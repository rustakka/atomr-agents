//! Workflow data types + runner — DAG, step kinds, run outcomes,
//! Python-driven callables / branch predicates.
//!
//! What this module provides:
//!
//! - `StepKind` — the historical string-tagged step discriminator.
//! - `WorkflowRunner` — Python wrapper over [`atomr_agents_workflow::WorkflowRunner`]
//!   with a `from_dict` constructor and an awaitable `run(input)` method.
//! - `register_workflow_callable(key, target)` — register a Python callable
//!   under `key` so dict-shaped DAGs can refer to it from `Step::Invoke`.
//! - `register_workflow_predicate(key, target)` — same but for `Step::Branch`
//!   (the target must return a bool synchronously).
//!
//! Supported step kinds in `from_dict`:
//!   - `"invoke"` — wires `Step::Invoke { callable, .. }` from the
//!     `CALLABLE_REGISTRY` keyed by `callable_key`.
//!   - `"branch"` — wires `Step::Branch { predicate, if_true, if_false }`
//!     from the `PREDICATE_REGISTRY` keyed by `predicate_key`.
//!
//! Deferred (return `ValueError`): `parallel`, `loop`, `map`, `human`.
//! These are either v0 stubs in the Rust runner today
//! (`workflow/src/runner.rs::exec_step`) or non-trivial to map from a
//! flat dict; they'll land alongside the proper Rust implementations.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::{Callable, CallableHandle};
use atomr_agents_core::{CallCtx, Result as AgentResult, Value, WorkflowId};
use atomr_agents_workflow::{BranchPredicate, Dag, InMemoryJournal, Journal, Step, StepId, WorkflowRunner};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use pyo3::exceptions::{PyKeyError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::conv::{json_to_py, py_to_json};

// ----- StepKind (historical data type) ---------------------------------------

#[pyclass(name = "StepKind", module = "atomr_agents._native.workflow", eq, hash, frozen)]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PyStepKind {
    inner: String,
}

#[pymethods]
impl PyStepKind {
    #[new]
    fn new(name: &str) -> PyResult<Self> {
        let valid = ["invoke", "branch", "parallel", "loop", "map", "human"];
        if !valid.contains(&name) {
            return Err(PyValueError::new_err(format!("unknown step kind: {name:?}")));
        }
        Ok(Self {
            inner: name.to_string(),
        })
    }

    #[getter]
    fn name(&self) -> &str {
        &self.inner
    }

    #[staticmethod]
    fn invoke() -> Self {
        Self {
            inner: "invoke".to_string(),
        }
    }
    #[staticmethod]
    fn branch() -> Self {
        Self {
            inner: "branch".to_string(),
        }
    }
    #[staticmethod]
    fn parallel() -> Self {
        Self {
            inner: "parallel".to_string(),
        }
    }
    #[staticmethod]
    fn loop_() -> Self {
        Self {
            inner: "loop".to_string(),
        }
    }
    #[staticmethod]
    fn map() -> Self {
        Self {
            inner: "map".to_string(),
        }
    }
    #[staticmethod]
    fn human() -> Self {
        Self {
            inner: "human".to_string(),
        }
    }

    fn __repr__(&self) -> String {
        format!("StepKind({:?})", self.inner)
    }
}

// ----- Process-wide registries ----------------------------------------------
//
// We keep these separate from `guest::GUESTS` so that a future split of
// `guest.rs` into per-adapter modules (per the Phase C plan) doesn't
// generate merge conflicts with this worktree.

static CALLABLE_REGISTRY: Lazy<RwLock<HashMap<String, Arc<dyn Callable>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

static PREDICATE_REGISTRY: Lazy<RwLock<HashMap<String, Arc<dyn BranchPredicate>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

#[pyfunction]
fn register_workflow_callable(key: String, target: PyObject) -> PyResult<()> {
    let adapter = PyCallableAdapter {
        target: Arc::new(target),
        label: key.clone(),
    };
    CALLABLE_REGISTRY.write().insert(key, Arc::new(adapter));
    Ok(())
}

#[pyfunction]
fn register_workflow_predicate(key: String, target: PyObject) -> PyResult<()> {
    let adapter = PyBranchPredicateAdapter {
        target: Arc::new(target),
    };
    PREDICATE_REGISTRY.write().insert(key, Arc::new(adapter));
    Ok(())
}

#[pyfunction]
fn clear_workflow_registries() -> usize {
    let n = CALLABLE_REGISTRY.read().len() + PREDICATE_REGISTRY.read().len();
    CALLABLE_REGISTRY.write().clear();
    PREDICATE_REGISTRY.write().clear();
    n
}

// ----- PyCallableAdapter -----------------------------------------------------
//
// Wraps a Python callable / class as a Rust `Callable`. Same shape as
// `PyToolAdapter` in guest.rs — instance-vs-class detection, coroutine
// detection via `inspect.iscoroutine`, awaiting via `into_future`,
// JSON round-trip on the input/output Value.

struct PyCallableAdapter {
    target: Arc<PyObject>,
    label: String,
}

#[async_trait]
impl Callable for PyCallableAdapter {
    async fn call(&self, input: Value, _ctx: CallCtx) -> AgentResult<Value> {
        let target = self.target.clone();
        // 1. Acquire GIL, build Python args, invoke `.call(input, ctx)`.
        //    `ctx` is projected as an empty dict for now — the workflow
        //    runner ignores it and constructs its own default context.
        let returned = Python::with_gil(|py| -> PyResult<PyObject> {
            let input_obj = json_to_py(py, &input)?;
            let ctx_dict = PyDict::new_bound(py);
            let bound = target.bind(py);
            // If the target exposes `.call`, prefer that (instance form).
            // Otherwise treat the target as directly callable (function or
            // class with `__call__`); if it's a class, instantiate it
            // first (zero-arg ctor), then call its `call` method.
            let instance: Bound<'_, PyAny> = if bound.hasattr("call")? {
                bound.clone()
            } else if bound.is_callable() {
                let inst = bound.call0()?;
                inst
            } else {
                bound.clone()
            };
            let call_attr = instance.getattr("call")?;
            let result = call_attr.call1((input_obj, ctx_dict))?;
            Ok(result.unbind())
        })
        .map_err(|e| atomr_agents_core::AgentError::Workflow(format!("guest callable: {e}")))?;

        // 2. If the return is a coroutine, await it.
        let final_val = {
            let maybe_future = Python::with_gil(|py| -> PyResult<Option<_>> {
                let bound = returned.bind(py);
                let inspect = py.import_bound("inspect")?;
                let iscoroutine = inspect.getattr("iscoroutine")?;
                let is_coro: bool = iscoroutine.call1((bound,))?.extract()?;
                if is_coro {
                    let fut = pyo3_async_runtimes::tokio::into_future(bound.clone())?;
                    Ok(Some(fut))
                } else {
                    Ok(None)
                }
            })
            .map_err(|e| atomr_agents_core::AgentError::Workflow(format!("guest callable coroutine: {e}")))?;

            match maybe_future {
                Some(fut) => fut.await.map_err(|e| {
                    atomr_agents_core::AgentError::Workflow(format!("guest callable await: {e}"))
                })?,
                None => returned,
            }
        };

        // 3. Convert the Python result back to JSON.
        let v = Python::with_gil(|py| py_to_json(py, final_val.bind(py)))
            .map_err(|e| atomr_agents_core::AgentError::Workflow(format!("guest callable result: {e}")))?;
        Ok(v)
    }

    fn label(&self) -> &str {
        &self.label
    }
}

// ----- PyBranchPredicateAdapter ---------------------------------------------
//
// `BranchPredicate::evaluate` is sync, so we just acquire the GIL,
// project the value as a Python object, and call `.evaluate(value)`
// (or the target directly if it's callable). Returns `false` on any
// Python error so a misbehaving predicate doesn't tank the workflow.

struct PyBranchPredicateAdapter {
    target: Arc<PyObject>,
}

impl BranchPredicate for PyBranchPredicateAdapter {
    fn evaluate(&self, value: &Value) -> bool {
        let target = self.target.clone();
        Python::with_gil(|py| -> PyResult<bool> {
            let value_obj = json_to_py(py, value)?;
            let bound = target.bind(py);
            // Prefer `.evaluate(value)` on an instance, else call the
            // target directly (function or class instance).
            let result = if bound.hasattr("evaluate")? {
                let attr = bound.getattr("evaluate")?;
                attr.call1((value_obj,))?
            } else if bound.is_callable() {
                bound.call1((value_obj,))?
            } else {
                return Ok(false);
            };
            result.extract::<bool>()
        })
        .unwrap_or(false)
    }
}

// ----- PyWorkflowRunner ------------------------------------------------------

#[pyclass(name = "WorkflowRunner", module = "atomr_agents._native.workflow")]
pub struct PyWorkflowRunner {
    pub(crate) inner: Arc<WorkflowRunner>,
}

#[pymethods]
impl PyWorkflowRunner {
    /// Build a workflow from a Python dict-shaped DAG description.
    ///
    /// Spec format:
    ///
    /// ```text
    /// {
    ///   "id": "wf-xxx",
    ///   "entry": "step_id",
    ///   "steps": {
    ///     "step_id": {"kind": "invoke", "callable_key": "echo"},
    ///     "step_b":  {"kind": "branch",
    ///                  "predicate_key": "is_even",
    ///                  "if_true": "step_c",
    ///                  "if_false": "step_d"},
    ///   },
    ///   "edges": [["from_id", "to_id"], ...]
    /// }
    /// ```
    ///
    /// `parallel` / `loop` / `map` / `human` step kinds raise
    /// `ValueError` for now.
    #[staticmethod]
    fn from_dict(spec: &Bound<'_, PyDict>) -> PyResult<Self> {
        // 1. Extract top-level fields.
        let id_str: String = spec
            .get_item("id")?
            .ok_or_else(|| PyKeyError::new_err("workflow spec: missing 'id'"))?
            .extract()?;
        let entry_str: String = spec
            .get_item("entry")?
            .ok_or_else(|| PyKeyError::new_err("workflow spec: missing 'entry'"))?
            .extract()?;
        let steps_obj = spec
            .get_item("steps")?
            .ok_or_else(|| PyKeyError::new_err("workflow spec: missing 'steps'"))?;
        let steps_dict = steps_obj
            .downcast::<PyDict>()
            .map_err(|_| PyValueError::new_err("workflow spec: 'steps' must be a dict"))?;
        let edges_obj = spec.get_item("edges")?;

        // 2. Build the DAG.
        let mut builder = Dag::<Step>::builder(entry_str.as_str());

        let callables = CALLABLE_REGISTRY.read();
        let predicates = PREDICATE_REGISTRY.read();

        for (key, value) in steps_dict.iter() {
            let step_id: String = key.extract()?;
            let step_dict = value.downcast::<PyDict>().map_err(|_| {
                PyValueError::new_err(format!("workflow spec: step {step_id:?} must map to a dict"))
            })?;
            let kind: String = step_dict
                .get_item("kind")?
                .ok_or_else(|| {
                    PyKeyError::new_err(format!("workflow spec: step {step_id:?} missing 'kind'"))
                })?
                .extract()?;

            let step = match kind.as_str() {
                "invoke" => {
                    let callable_key: String = step_dict
                        .get_item("callable_key")?
                        .ok_or_else(|| {
                            PyKeyError::new_err(format!(
                                "workflow spec: invoke step {step_id:?} missing 'callable_key'"
                            ))
                        })?
                        .extract()?;
                    let handle = callables.get(&callable_key).cloned().ok_or_else(|| {
                        PyKeyError::new_err(format!(
                            "no workflow callable registered with key {callable_key:?}"
                        ))
                    })?;
                    let handle: CallableHandle = handle;
                    Step::invoke(handle)
                }
                "branch" => {
                    let predicate_key: String = step_dict
                        .get_item("predicate_key")?
                        .ok_or_else(|| {
                            PyKeyError::new_err(format!(
                                "workflow spec: branch step {step_id:?} missing 'predicate_key'"
                            ))
                        })?
                        .extract()?;
                    let if_true: String = step_dict
                        .get_item("if_true")?
                        .ok_or_else(|| {
                            PyKeyError::new_err(format!(
                                "workflow spec: branch step {step_id:?} missing 'if_true'"
                            ))
                        })?
                        .extract()?;
                    let if_false: String = step_dict
                        .get_item("if_false")?
                        .ok_or_else(|| {
                            PyKeyError::new_err(format!(
                                "workflow spec: branch step {step_id:?} missing 'if_false'"
                            ))
                        })?
                        .extract()?;
                    let predicate = predicates.get(&predicate_key).cloned().ok_or_else(|| {
                        PyKeyError::new_err(format!(
                            "no workflow predicate registered with key {predicate_key:?}"
                        ))
                    })?;
                    Step::Branch {
                        predicate,
                        if_true: StepId::new(if_true),
                        if_false: StepId::new(if_false),
                    }
                }
                other @ ("parallel" | "loop" | "map" | "human") => {
                    return Err(PyValueError::new_err(format!(
                        "step kind {other:?} not yet supported in Python wrapper (Phase C v0)"
                    )));
                }
                other => {
                    return Err(PyValueError::new_err(format!("unknown step kind {other:?}")));
                }
            };

            builder = builder.step(step_id.as_str(), step);
        }

        // 3. Edges.
        if let Some(edges_obj) = edges_obj {
            let edges_list = edges_obj.downcast::<PyList>().map_err(|_| {
                PyValueError::new_err("workflow spec: 'edges' must be a list of [from, to] pairs")
            })?;
            for edge in edges_list.iter() {
                let pair = edge.downcast::<PyList>().map_err(|_| {
                    PyValueError::new_err("workflow spec: each edge must be a [from, to] pair")
                })?;
                if pair.len() != 2 {
                    return Err(PyValueError::new_err(
                        "workflow spec: each edge must be a 2-element [from, to] pair",
                    ));
                }
                let from: String = pair.get_item(0)?.extract()?;
                let to: String = pair.get_item(1)?.extract()?;
                builder = builder.edge(from.as_str(), to.as_str());
            }
        }

        let dag = builder.build();
        let journal: Arc<dyn Journal> = Arc::new(InMemoryJournal::new());
        let runner = WorkflowRunner::new(WorkflowId::from(id_str), dag, journal);

        Ok(PyWorkflowRunner {
            inner: Arc::new(runner),
        })
    }

    /// Run the workflow with a JSON-serializable input. Returns a
    /// Python awaitable that resolves to the final step output (also
    /// JSON-serializable).
    fn run<'py>(&self, py: Python<'py>, input: PyObject) -> PyResult<Bound<'py, PyAny>> {
        let inp = py_to_json(py, input.bind(py))?;
        let runner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let out = runner
                .run(inp)
                .await
                .map_err(|e| PyErr::new::<crate::errors::WorkflowError, _>(e.to_string()))?;
            Python::with_gil(|py| json_to_py(py, &out))
        })
    }
}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "workflow")?;
    m.add_class::<PyStepKind>()?;
    m.add_class::<PyWorkflowRunner>()?;
    m.add_function(wrap_pyfunction!(register_workflow_callable, &m)?)?;
    m.add_function(wrap_pyfunction!(register_workflow_predicate, &m)?)?;
    m.add_function(wrap_pyfunction!(clear_workflow_registries, &m)?)?;
    parent.add_submodule(&m)?;
    Ok(())
}

// NOTE: This crate is built with `pyo3/extension-module`, which omits
// the libpython link arguments. As a result `cargo test -p
// atomr-agents-py-bindings` cannot link a test binary (every other
// `_native` symbol is undefined). The whole crate intentionally has no
// `#[test]` modules — Python-side integration tests under
// `python/atomr_agents/tests/` exercise the bindings post-`maturin
// develop`. The new `from_dict` / `run` surface is verified by:
//
//   1. The workflow crate's `WorkflowRunner::run` tests
//      (`crates/workflow/src/runner.rs::tests`), which cover the Rust
//      runner — including the new `WorkflowRunner::new` constructor.
//   2. A Python smoke test (see the W5 spec) registering an echo
//      callable, building a 2-step DAG, and `await`-ing
//      `runner.run(input)`.
