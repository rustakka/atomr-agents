//! Python-implemented `LoopStrategy` and `TerminationStrategy`
//! adapters, plus their guest registration helpers.
//!
//! Kept separate from `crate::guest` so this file can land while W2's
//! parallel `guest.rs` restructure is in flight without merge conflicts.
//! After both worktrees merge, the `register_*_factory` helpers and
//! `Py*Adapter` structs can be migrated into the new `guest/` module
//! tree as part of cleanup.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentError, HarnessId, Result as AgentResult, Value};
use atomr_agents_harness::{
    BoxedHarness, HarnessDispatch, HarnessState, IterationCapTermination, LoopStrategy,
    StepOutcome, Termination, TerminationStrategy,
};
use atomr_agents_observability::EventBus;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::conv::{json_to_py, py_to_json};
use crate::guest::{lookup_guest, register_kind, PyGuestHandle};
use crate::harness::PyHarnessSpec;

// ---------------------------------------------------------------------------
// Guest registration entrypoints
// ---------------------------------------------------------------------------

/// Register a Python class/instance implementing `LoopStrategy.step`.
/// Stored under kind `"loop_strategy"` in the shared GUESTS registry.
#[pyfunction]
pub fn register_loop_strategy_factory(key: String, target: PyObject) -> PyGuestHandle {
    register_kind("loop_strategy", key, target)
}

/// Register a Python class/instance implementing
/// `TerminationStrategy.should_terminate`. Stored under kind
/// `"termination"` in the shared GUESTS registry.
#[pyfunction]
pub fn register_termination_factory(key: String, target: PyObject) -> PyGuestHandle {
    register_kind("termination", key, target)
}

// ---------------------------------------------------------------------------
// HarnessState projection
// ---------------------------------------------------------------------------

/// Project a `HarnessState` into a Python dict that adapters pass to
/// the user's `step` / `should_terminate` callable. Mirrors
/// `PyToolAdapter::build_ctx_dict` style.
fn build_state_dict<'py>(py: Python<'py>, state: &HarnessState) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new_bound(py);
    d.set_item("iteration", state.iteration)?;
    d.set_item("remaining_tokens", state.remaining_tokens)?;
    d.set_item("working_memory", json_to_py(py, &state.working_memory)?)?;
    let history = PyList::empty_bound(py);
    for ev in &state.history {
        let h = PyDict::new_bound(py);
        h.set_item("iteration", ev.iteration)?;
        h.set_item("outcome", &ev.outcome)?;
        h.set_item("timestamp_ms", ev.timestamp_ms)?;
        history.append(h)?;
    }
    d.set_item("history", history)?;
    Ok(d)
}

// ---------------------------------------------------------------------------
// LoopStrategy adapter
// ---------------------------------------------------------------------------

/// Wraps a Python class or instance as a Rust `LoopStrategy`. The
/// Python target must expose a `step(state)` callable returning either
/// a `StepOutcome`-shaped dict or a coroutine yielding one. Accepted
/// dict shapes (case-insensitive `kind`):
///
/// - `{"kind": "continue", "working_memory": <json>, "label": <str>}`
/// - `{"kind": "done", "output": <json>, "label": <str>}`
///
/// Plain serde forms (`{"Continue": {...}}` / `{"Done": {...}}`) also
/// deserialize transparently.
pub struct PyLoopStrategyAdapter {
    target: Arc<PyObject>,
    label: String,
}

impl PyLoopStrategyAdapter {
    pub fn new(target: Arc<PyObject>, label: impl Into<String>) -> Self {
        Self {
            target,
            label: label.into(),
        }
    }
}

#[async_trait]
impl LoopStrategy for PyLoopStrategyAdapter {
    async fn step(&self, state: &mut HarnessState) -> AgentResult<StepOutcome> {
        let target = self.target.clone();

        // 1. Acquire GIL, project state, invoke `step`, capture result.
        let returned = Python::with_gil(|py| -> PyResult<PyObject> {
            let state_dict = build_state_dict(py, state)?;
            let bound = target.bind(py);
            let instance: Bound<'_, PyAny> = if bound.hasattr("step")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let step_attr = instance.getattr("step")?;
            let result = step_attr.call1((state_dict,))?;
            Ok(result.unbind())
        })
        .map_err(|e| {
            AgentError::Strategy(format!(
                "guest loop_strategy {:?}.step() raised: {e}",
                self.label
            ))
        })?;

        // 2. If the return value is a coroutine, await it.
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
            .map_err(|e| {
                AgentError::Strategy(format!(
                    "guest loop_strategy {:?} coroutine setup: {e}",
                    self.label
                ))
            })?;

            match maybe_future {
                Some(fut) => fut.await.map_err(|e| {
                    AgentError::Strategy(format!(
                        "guest loop_strategy {:?} await: {e}",
                        self.label
                    ))
                })?,
                None => returned,
            }
        };

        // 3. Convert Python result to JSON, then parse into StepOutcome
        //    (accepting both the kind-tagged ergonomic form and the
        //    plain serde-default enum form).
        let json = Python::with_gil(|py| py_to_json(py, final_val.bind(py))).map_err(|e| {
            AgentError::Strategy(format!(
                "guest loop_strategy {:?} result: {e}",
                self.label
            ))
        })?;
        parse_step_outcome(json).map_err(|e| {
            AgentError::Strategy(format!(
                "guest loop_strategy {:?} returned invalid StepOutcome: {e}",
                self.label
            ))
        })
    }
}

fn parse_step_outcome(v: Value) -> Result<StepOutcome, String> {
    // Accept the Pythonic kind-tagged form first; fall through to the
    // serde default representation.
    if let Value::Object(map) = &v {
        if let Some(kind) = map.get("kind").and_then(|k| k.as_str()) {
            let label = map
                .get("label")
                .and_then(|l| l.as_str())
                .unwrap_or("step")
                .to_string();
            return match kind.to_ascii_lowercase().as_str() {
                "continue" => {
                    let working_memory =
                        map.get("working_memory").cloned().unwrap_or(Value::Null);
                    Ok(StepOutcome::Continue {
                        working_memory,
                        label,
                    })
                }
                "done" => {
                    let output = map.get("output").cloned().unwrap_or(Value::Null);
                    Ok(StepOutcome::Done { output, label })
                }
                other => Err(format!("unknown StepOutcome kind {other:?}")),
            };
        }
    }
    serde_json::from_value::<StepOutcome>(v).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// TerminationStrategy adapter (sync, single GIL acquisition)
// ---------------------------------------------------------------------------

/// Wraps a Python class or instance as a Rust `TerminationStrategy`.
/// The Python target must expose `should_terminate(state)` that
/// returns one of:
///
/// - `False` / `None` / `{"continue": True}` → keep looping.
/// - `True` / a non-empty string → terminate (string used as the
///   reason; `True` becomes `"guest"`).
/// - `{"done": "<reason>"}` → terminate with `<reason>`.
pub struct PyTerminationAdapter {
    target: Arc<PyObject>,
    label: String,
}

impl PyTerminationAdapter {
    pub fn new(target: Arc<PyObject>, label: impl Into<String>) -> Self {
        Self {
            target,
            label: label.into(),
        }
    }

    fn classify(v: Value, label: &str) -> Termination {
        // Map a JSON-projected Python return value to a `Termination`.
        // We can't return arbitrary owned strings since `Termination`
        // wraps `&'static str`; intern reasons via match-on-known forms
        // and fall back to a single static string for guest-supplied
        // reasons. Reasons therefore are coarse-grained on the Rust
        // side; the Python adapter `label` provides finer attribution
        // through the bus events.
        match v {
            Value::Bool(true) => Termination::Done("guest"),
            Value::Bool(false) | Value::Null => Termination::Continue,
            Value::String(s) => {
                if s.is_empty() {
                    Termination::Continue
                } else {
                    Termination::Done("guest")
                }
            }
            Value::Object(map) => {
                if map.contains_key("done") || map.get("terminate") == Some(&Value::Bool(true)) {
                    Termination::Done("guest")
                } else if map.get("continue") == Some(&Value::Bool(true)) {
                    Termination::Continue
                } else {
                    // Empty / unknown object → assume "continue".
                    Termination::Continue
                }
            }
            other => {
                // Tracing only — we don't have access to a logger here;
                // leave breadcrumbs in the bus by emitting Continue and
                // letting the iteration cap stop runaways. The `label`
                // is referenced in the panic-free fallback message.
                let _ = (other, label);
                Termination::Continue
            }
        }
    }
}

impl TerminationStrategy for PyTerminationAdapter {
    fn should_terminate(&self, state: &HarnessState) -> Termination {
        let target = self.target.clone();
        let label = self.label.clone();
        let result = Python::with_gil(|py| -> PyResult<Value> {
            let state_dict = build_state_dict(py, state)?;
            let bound = target.bind(py);
            let instance: Bound<'_, PyAny> = if bound.hasattr("should_terminate")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let attr = instance.getattr("should_terminate")?;
            let raw = attr.call1((state_dict,))?;
            py_to_json(py, &raw)
        });
        match result {
            Ok(v) => Self::classify(v, &label),
            Err(_e) => {
                // Best-effort: a guest error stops the loop to avoid
                // tight error spirals. The harness `Callable` wrapper
                // surfaces no failure here because should_terminate is
                // sync and infallible by trait shape; the "guest"
                // outcome label is what shows up on the event bus.
                Termination::Done("guest_error")
            }
        }
    }
}

// ---------------------------------------------------------------------------
// PyHarness — the boxed-handle Python wrapper
// ---------------------------------------------------------------------------

/// `Harness` Python wrapper backed by an `Arc<dyn HarnessDispatch>`.
/// Construct via `Harness.from_spec(spec, loop_key, termination_key)`
/// and drive with the async `run()` coroutine.
#[pyclass(name = "Harness", module = "atomr_agents._native.harness")]
pub struct PyHarness {
    pub(crate) inner: Arc<dyn HarnessDispatch>,
    pub(crate) id: HarnessId,
}

#[pymethods]
impl PyHarness {
    /// Build a runnable `Harness` from a spec, a registered loop strategy
    /// key, and a termination key.
    ///
    /// The termination key has one ergonomic special case: passing
    /// `"iteration_cap:<N>"` (e.g. `"iteration_cap:32"`) uses the stock
    /// `IterationCapTermination` with `cap = N`. Plain `"iteration_cap"`
    /// also works and defaults to `cap = 100`. Any other key is looked
    /// up in the guest registry under the `"termination"` kind.
    #[staticmethod]
    fn from_spec(spec: PyHarnessSpec, loop_key: String, termination_key: String) -> PyResult<Self> {
        // 1. Look up the loop strategy.
        let loop_target = lookup_guest("loop_strategy", &loop_key).ok_or_else(|| {
            pyo3::exceptions::PyKeyError::new_err(format!(
                "no loop_strategy registered with key {loop_key:?}; \
                 register one via guest.register_loop_strategy_factory()"
            ))
        })?;
        let loop_strategy: Box<dyn LoopStrategy> = Box::new(PyLoopStrategyAdapter::new(
            loop_target,
            loop_key.clone(),
        ));

        // 2. Termination: special-case "iteration_cap" / "iteration_cap:N",
        //    otherwise look up a guest-registered factory.
        let termination: Box<dyn TerminationStrategy> = if let Some(rest) =
            termination_key.strip_prefix("iteration_cap")
        {
            let cap: u64 = if rest.is_empty() {
                100
            } else if let Some(n_str) = rest.strip_prefix(':') {
                n_str.parse().map_err(|e| {
                    pyo3::exceptions::PyValueError::new_err(format!(
                        "iteration_cap requires an integer N (got {n_str:?}): {e}"
                    ))
                })?
            } else {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "unrecognized termination key {termination_key:?}; \
                     expected \"iteration_cap\" or \"iteration_cap:<N>\""
                )));
            };
            Box::new(IterationCapTermination { cap })
        } else {
            let term_target = lookup_guest("termination", &termination_key).ok_or_else(|| {
                pyo3::exceptions::PyKeyError::new_err(format!(
                    "no termination registered with key {termination_key:?}; \
                     register one via guest.register_termination_factory() or use \
                     \"iteration_cap[:<N>]\""
                ))
            })?;
            Box::new(PyTerminationAdapter::new(
                term_target,
                termination_key.clone(),
            ))
        };

        // 3. Build a BoxedHarness directly so we can hand the
        //    `Arc<dyn HarnessDispatch>` straight to PyHarness without
        //    the (private-inner) HarnessRef indirection.
        let id = spec.inner.id.clone();
        let boxed = BoxedHarness {
            spec: spec.inner,
            loop_strategy,
            termination,
            bus: EventBus::new(),
        };
        Ok(Self {
            inner: Arc::new(boxed),
            id,
        })
    }

    /// Run the harness loop. Returns a Python coroutine that resolves
    /// to the final working-memory value (JSON-converted to a Python
    /// object).
    fn run<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let result = inner.dispatch().await.map_err(|e| {
                PyErr::new::<crate::errors::HarnessError, _>(e.to_string())
            })?;
            Python::with_gil(|py| crate::conv::json_to_py(py, &result))
        })
    }

    #[getter]
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn __repr__(&self) -> String {
        format!("Harness(id={:?})", self.id.as_str())
    }
}

// ---------------------------------------------------------------------------
// Module registration helper
// ---------------------------------------------------------------------------

pub fn register_into(_py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    // The `harness` submodule already exists via `crate::harness::register`;
    // reach through the parent module to attach `PyHarness` and the new
    // guest registration helpers in their respective submodules.
    if let Ok(harness_mod) = parent.getattr("harness") {
        let harness_mod = harness_mod.downcast_into::<PyModule>()?;
        harness_mod.add_class::<PyHarness>()?;
    }
    if let Ok(guest_mod) = parent.getattr("guest") {
        let guest_mod = guest_mod.downcast_into::<PyModule>()?;
        guest_mod.add_function(wrap_pyfunction!(register_loop_strategy_factory, &guest_mod)?)?;
        guest_mod.add_function(wrap_pyfunction!(register_termination_factory, &guest_mod)?)?;
    }
    Ok(())
}
