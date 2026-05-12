//! Workflow runtime — `Dag`, `Step`, `WorkflowRunner`, journal,
//! interrupt API.
//!
//! Every step's body is a `PyCallable`, so the workflow doesn't need
//! to know how the callable was built (agent, pipeline, raw fn).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::CallableHandle;
use atomr_agents_core::{AgentError, Result as AgentResult, Value, WorkflowId};
use atomr_agents_workflow::{
    dispatch_fan_out, BranchPredicate, Concurrency, Dag, HumanApproval, InMemoryJournal,
    InputMapping, JoinStrategy, Journal, Step, StepId, WorkflowEvent, WorkflowRunner,
    WorkflowState,
};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::callable::PyCallable;
use crate::conv::{json_to_py, py_to_json};
use crate::strategy::await_if_coro;

// ----- StepKind (legacy / discriminator) ---------------------------------

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
            inner: "invoke".into(),
        }
    }
    #[staticmethod]
    fn branch() -> Self {
        Self {
            inner: "branch".into(),
        }
    }
    #[staticmethod]
    fn parallel() -> Self {
        Self {
            inner: "parallel".into(),
        }
    }
    #[staticmethod]
    fn loop_() -> Self {
        Self {
            inner: "loop".into(),
        }
    }
    #[staticmethod]
    fn map() -> Self {
        Self {
            inner: "map".into(),
        }
    }
    #[staticmethod]
    fn human() -> Self {
        Self {
            inner: "human".into(),
        }
    }

    fn __repr__(&self) -> String {
        format!("StepKind({:?})", self.inner)
    }
}

// ----- StepId ------------------------------------------------------------

#[pyclass(name = "StepId", module = "atomr_agents._native.workflow", eq, hash, frozen)]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PyStepId {
    pub(crate) inner: StepId,
}

#[pymethods]
impl PyStepId {
    #[new]
    fn new(value: String) -> Self {
        Self {
            inner: StepId::new(value),
        }
    }

    #[getter]
    fn value(&self) -> &str {
        self.inner.as_str()
    }

    fn __repr__(&self) -> String {
        format!("StepId({:?})", self.inner.as_str())
    }
}

// ----- Step (enum wrapper) -----------------------------------------------

/// Step body. Stored opaquely so the construction-site factory can be
/// matched on later. Python sees a `Step` class with classmethods.
#[pyclass(name = "Step", module = "atomr_agents._native.workflow")]
pub struct PyStep {
    pub(crate) inner: Option<Step>,
}

#[pymethods]
impl PyStep {
    #[staticmethod]
    fn invoke(callable: PyCallable) -> Self {
        Self {
            inner: Some(Step::Invoke {
                callable: callable.inner,
                mapping: InputMapping::default(),
            }),
        }
    }

    /// `Step.invoke_with_mapping(callable, fields=[...])` — restrict
    /// the workflow input projected into the callable to the named
    /// fields.
    #[staticmethod]
    #[pyo3(signature = (callable, fields=Vec::new()))]
    fn invoke_with_mapping(callable: PyCallable, fields: Vec<String>) -> Self {
        Self {
            inner: Some(Step::Invoke {
                callable: callable.inner,
                mapping: InputMapping { fields },
            }),
        }
    }

    /// `Step.branch(predicate, if_true, if_false)` — predicate is a
    /// Python callable taking the previous step's output value.
    #[staticmethod]
    fn branch(predicate: PyObject, if_true: PyStepId, if_false: PyStepId) -> Self {
        let pred = Arc::new(predicate);
        struct PyPredicate(Arc<PyObject>);
        impl BranchPredicate for PyPredicate {
            fn evaluate(&self, output: &Value) -> bool {
                let p = self.0.clone();
                Python::with_gil(|py| {
                    let bound = p.bind(py);
                    let arg = match json_to_py(py, output) {
                        Ok(o) => o,
                        Err(_) => return false,
                    };
                    bound
                        .call1((arg.bind(py),))
                        .and_then(|r| r.is_truthy())
                        .unwrap_or(false)
                })
            }
        }
        Self {
            inner: Some(Step::Branch {
                predicate: Arc::new(PyPredicate(pred)),
                if_true: if_true.inner,
                if_false: if_false.inner,
            }),
        }
    }

    /// `Step.parallel(steps, join="all")` — `join` is "all" or "any".
    #[staticmethod]
    #[pyo3(signature = (steps, join="all".to_string()))]
    fn parallel(steps: Vec<PyStepId>, join: String) -> PyResult<Self> {
        let join = match join.as_str() {
            "all" => JoinStrategy::All,
            "any" => JoinStrategy::Any,
            other => {
                return Err(PyValueError::new_err(format!(
                    "join must be 'all' or 'any', got {other:?}"
                )));
            }
        };
        Ok(Self {
            inner: Some(Step::Parallel {
                steps: steps.into_iter().map(|s| s.inner).collect(),
                join,
            }),
        })
    }

    /// `Step.loop_(body, predicate)` — repeats `body` while predicate
    /// is true on its output.
    #[staticmethod]
    fn loop_(body: PyStepId, predicate: PyObject) -> Self {
        let pred = Arc::new(predicate);
        struct PyPredicate(Arc<PyObject>);
        impl BranchPredicate for PyPredicate {
            fn evaluate(&self, output: &Value) -> bool {
                let p = self.0.clone();
                Python::with_gil(|py| {
                    let bound = p.bind(py);
                    let arg = match json_to_py(py, output) {
                        Ok(o) => o,
                        Err(_) => return false,
                    };
                    bound
                        .call1((arg.bind(py),))
                        .and_then(|r| r.is_truthy())
                        .unwrap_or(false)
                })
            }
        }
        Self {
            inner: Some(Step::Loop {
                body: body.inner,
                predicate: Arc::new(PyPredicate(pred)),
            }),
        }
    }

    /// `Step.map(body, concurrency=1)` — fan out `body` over input array.
    #[staticmethod]
    #[pyo3(signature = (body, concurrency=1))]
    fn map(body: PyStepId, concurrency: u32) -> Self {
        Self {
            inner: Some(Step::Map {
                body: body.inner,
                concurrency: Concurrency(concurrency),
            }),
        }
    }

    /// `Step.human(prompt, context=None)` — pause for human approval.
    #[staticmethod]
    #[pyo3(signature = (prompt, context=None))]
    fn human(py: Python<'_>, prompt: String, context: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let context = match context {
            Some(c) if !c.is_none() => py_to_json(py, c)?,
            _ => Value::Null,
        };
        Ok(Self {
            inner: Some(Step::Human {
                approval: HumanApproval { prompt, context },
            }),
        })
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            Some(Step::Invoke { .. }) => "Step(invoke)".into(),
            Some(Step::Branch { .. }) => "Step(branch)".into(),
            Some(Step::Parallel { .. }) => "Step(parallel)".into(),
            Some(Step::Loop { .. }) => "Step(loop)".into(),
            Some(Step::Map { .. }) => "Step(map)".into(),
            Some(Step::Human { .. }) => "Step(human)".into(),
            None => "Step(consumed)".into(),
        }
    }
}

// ----- Dag builder -------------------------------------------------------

#[pyclass(name = "Dag", module = "atomr_agents._native.workflow")]
pub struct PyDag {
    builder: Option<DagBuilder>,
}

struct DagBuilder {
    steps: std::collections::BTreeMap<StepId, Step>,
    edges: HashMap<StepId, Vec<StepId>>,
    entry: StepId,
}

#[pymethods]
impl PyDag {
    /// `Dag(entry_id)` — start a fresh DAG with the given entry step.
    #[new]
    fn new(entry: String) -> Self {
        Self {
            builder: Some(DagBuilder {
                steps: std::collections::BTreeMap::new(),
                edges: HashMap::new(),
                entry: StepId::new(entry),
            }),
        }
    }

    fn add_step(&mut self, id: String, step: &mut PyStep) -> PyResult<()> {
        let b = self
            .builder
            .as_mut()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("dag already built"))?;
        let s = step
            .inner
            .take()
            .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("step has already been used"))?;
        b.steps.insert(StepId::new(id), s);
        Ok(())
    }

    fn add_edge(&mut self, from: String, to: String) -> PyResult<()> {
        let b = self
            .builder
            .as_mut()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("dag already built"))?;
        b.edges.entry(StepId::new(from)).or_default().push(StepId::new(to));
        Ok(())
    }

    fn set_entry(&mut self, id: String) -> PyResult<()> {
        let b = self
            .builder
            .as_mut()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("dag already built"))?;
        b.entry = StepId::new(id);
        Ok(())
    }

    /// Materialise into a `Dag<Step>`. Workflow runners take ownership.
    fn build(&mut self) -> PyResult<PyDagHandle> {
        let b = self
            .builder
            .take()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("dag already built"))?;
        Ok(PyDagHandle {
            inner: Some(Arc::new(Dag {
                steps: b.steps,
                edges: b.edges,
                entry: b.entry,
            })),
        })
    }
}

/// Opaque handle to a built `Dag<Step>`. Stored as an `Arc` so it can
/// be cloned into a `WorkflowRunner`.
#[pyclass(name = "DagHandle", module = "atomr_agents._native.workflow")]
#[derive(Clone)]
pub struct PyDagHandle {
    pub(crate) inner: Option<Arc<Dag<Step>>>,
}

// ----- Journal -----------------------------------------------------------

#[pyclass(name = "Journal", module = "atomr_agents._native.workflow")]
#[derive(Clone)]
pub struct PyJournal {
    pub(crate) inner: Arc<dyn Journal>,
}

#[pymethods]
impl PyJournal {
    fn __repr__(&self) -> String {
        "Journal(handle)".into()
    }
}

pub(crate) struct PyJournalAdapter {
    target: Arc<PyObject>,
}

#[async_trait]
impl Journal for PyJournalAdapter {
    async fn append(&self, workflow_id: &WorkflowId, event: WorkflowEvent) -> AgentResult<()> {
        let target = self.target.clone();
        let wid = workflow_id.as_str().to_string();
        let ev = serde_json::to_value(&event)
            .map_err(|e| AgentError::Workflow(format!("event serialize: {e}")))?;
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let ev_py = json_to_py(py, &ev)?;
            let instance: Bound<'_, PyAny> = if bound.hasattr("append")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("append")?.call1((wid, ev_py))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Workflow(format!("py journal append: {e}")))?;
        let _ = await_if_coro(coro_or_val).await?;
        Ok(())
    }

    async fn replay(&self, workflow_id: &WorkflowId) -> AgentResult<Vec<WorkflowEvent>> {
        let target = self.target.clone();
        let wid = workflow_id.as_str().to_string();
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let instance: Bound<'_, PyAny> = if bound.hasattr("replay")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("replay")?.call1((wid,))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Workflow(format!("py journal replay: {e}")))?;
        let final_val = await_if_coro(coro_or_val).await?;
        Python::with_gil(|py| -> PyResult<Vec<WorkflowEvent>> {
            let v = py_to_json(py, final_val.bind(py))?;
            serde_json::from_value::<Vec<WorkflowEvent>>(v)
                .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))
        })
        .map_err(|e| AgentError::Workflow(format!("py journal replay parse: {e}")))
    }
}

#[pyfunction]
fn in_memory_journal() -> PyJournal {
    PyJournal {
        inner: Arc::new(InMemoryJournal::new()),
    }
}

#[pyfunction]
fn journal_from_factory(key: String) -> PyResult<PyJournal> {
    let target = crate::guest::must_lookup("journal", &key)?;
    Ok(PyJournal {
        inner: Arc::new(PyJournalAdapter { target }),
    })
}

// ----- WorkflowState (data) ----------------------------------------------

#[pyclass(name = "WorkflowState", module = "atomr_agents._native.workflow")]
#[derive(Clone)]
pub struct PyWorkflowState {
    pub(crate) inner: WorkflowState,
}

#[pymethods]
impl PyWorkflowState {
    #[getter]
    fn completed(&self) -> Vec<String> {
        self.inner
            .completed
            .iter()
            .map(|s| s.as_str().to_string())
            .collect()
    }

    #[getter]
    fn terminated(&self) -> Option<bool> {
        self.inner.terminated
    }

    #[getter]
    fn outputs(&self, py: Python<'_>) -> PyResult<PyObject> {
        let dict = PyDict::new_bound(py);
        for (k, v) in &self.inner.outputs {
            dict.set_item(k.as_str(), json_to_py(py, v)?)?;
        }
        Ok(dict.unbind().into())
    }

    fn __repr__(&self) -> String {
        format!(
            "WorkflowState(completed={}, terminated={:?})",
            self.inner.completed.len(),
            self.inner.terminated
        )
    }
}

// ----- WorkflowRunner ----------------------------------------------------

#[pyclass(name = "WorkflowRunner", module = "atomr_agents._native.workflow")]
pub struct PyWorkflowRunner {
    pub(crate) inner: Arc<WorkflowRunner>,
}

#[pymethods]
impl PyWorkflowRunner {
    #[new]
    #[pyo3(signature = (id, dag, journal=None))]
    fn new(id: String, dag: &mut PyDagHandle, journal: Option<PyJournal>) -> PyResult<Self> {
        let dag_arc = dag
            .inner
            .take()
            .ok_or_else(|| PyValueError::new_err("dag handle already consumed"))?;
        let dag = Arc::try_unwrap(dag_arc).map_err(|_| {
            PyValueError::new_err("dag handle still referenced elsewhere — clone it first")
        })?;
        let journal: Arc<dyn Journal> = match journal {
            Some(j) => j.inner,
            None => Arc::new(InMemoryJournal::new()),
        };
        Ok(Self {
            inner: Arc::new(WorkflowRunner {
                id: WorkflowId::from(id),
                dag,
                journal,
            }),
        })
    }

    fn run<'py>(
        &self,
        py: Python<'py>,
        input: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let input_value = match input {
            Some(b) if !b.is_none() => py_to_json(py, b)?,
            _ => Value::Null,
        };
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let out = inner.run(input_value).await.map_err(crate::errors::map)?;
            Python::with_gil(|py| json_to_py(py, &out))
        })
    }

    fn as_callable(&self) -> PyCallable {
        // Wrap as a CallableHandle that takes the input value and runs.
        let inner = self.inner.clone();
        let h: CallableHandle = Arc::new(atomr_agents_callable::FnCallable::labeled(
            "workflow",
            move |v: Value, _ctx| {
                let inner = inner.clone();
                async move { inner.run(v).await }
            },
        ));
        PyCallable::from_handle(h)
    }

    #[getter]
    fn id(&self) -> &str {
        self.inner.id.as_str()
    }

    fn __repr__(&self) -> String {
        format!("WorkflowRunner(id={:?})", self.inner.id.as_str())
    }
}

// ----- dispatch_fan_out (no Subgraph in v0 — requires stateful runner) ----

/// Build a callable that runs `producer`, then dispatches its array
/// output through `target` with bounded concurrency. The resulting
/// callable accepts the seed input the producer should be invoked with.
#[pyfunction]
#[pyo3(signature = (producer, target, concurrency=1))]
fn fan_out_dispatch(
    producer: PyCallable,
    target: PyCallable,
    concurrency: u32,
) -> PyCallable {
    let producer_handle = producer.inner;
    let target_handle = target.inner;
    let h: CallableHandle = Arc::new(atomr_agents_callable::FnCallable::labeled(
        "fan_out_dispatch",
        move |seed: Value, ctx| {
            let producer = producer_handle.clone();
            let target = target_handle.clone();
            async move {
                let outs =
                    dispatch_fan_out(producer, target, concurrency, seed, ctx).await?;
                Ok(Value::Array(outs))
            }
        },
    ));
    PyCallable::from_handle(h)
}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "workflow")?;
    m.add_class::<PyStepKind>()?;
    m.add_class::<PyStepId>()?;
    m.add_class::<PyStep>()?;
    m.add_class::<PyDag>()?;
    m.add_class::<PyDagHandle>()?;
    m.add_class::<PyJournal>()?;
    m.add_class::<PyWorkflowState>()?;
    m.add_class::<PyWorkflowRunner>()?;
    m.add_function(wrap_pyfunction!(in_memory_journal, &m)?)?;
    m.add_function(wrap_pyfunction!(journal_from_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(fan_out_dispatch, &m)?)?;
    parent.add_submodule(&m)?;
    Ok(())
}
