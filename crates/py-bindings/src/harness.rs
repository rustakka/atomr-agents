//! Persistent harness loop.
//!
//! `Harness<L, T>` is generic over `LoopStrategy + TerminationStrategy`;
//! Python sees a `BoxedHarness`-style facade where the strategy slots
//! are `Box<dyn …>`.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::{Callable, CallableHandle};
use atomr_agents_core::{AgentError, CallCtx, HarnessId, Result as AgentResult, TokenBudget, Value};
use atomr_agents_harness::{
    Harness, HarnessSpec, HarnessState, IterationCapTermination, LoopStrategy, StepEvent, StepOutcome,
    Termination, TerminationStrategy,
};
use atomr_agents_observability::EventBus;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use semver::Version;

use crate::callable::PyCallable;
use crate::conv::{json_to_py, parse_version, py_to_json};
use crate::strategy::await_if_coro;

// ----- HarnessSpec --------------------------------------------------------

#[pyclass(name = "HarnessSpec", module = "atomr_agents._native.harness")]
#[derive(Clone)]
pub struct PyHarnessSpec {
    pub(crate) inner: HarnessSpec,
}

#[pymethods]
impl PyHarnessSpec {
    #[new]
    #[pyo3(signature = (id, version, initial_token_budget=8000, eval_suite_id=None))]
    fn new(
        id: String,
        version: &str,
        initial_token_budget: u32,
        eval_suite_id: Option<String>,
    ) -> PyResult<Self> {
        let v: Version = parse_version(version)?;
        Ok(Self {
            inner: HarnessSpec {
                id: HarnessId::from(id),
                version: v,
                eval_suite_id,
                initial_budget: TokenBudget::new(initial_token_budget),
            },
        })
    }

    #[getter]
    fn id(&self) -> &str {
        self.inner.id.as_str()
    }

    #[getter]
    fn version(&self) -> String {
        self.inner.version.to_string()
    }

    #[getter]
    fn eval_suite_id(&self) -> Option<String> {
        self.inner.eval_suite_id.clone()
    }

    #[getter]
    fn initial_token_budget(&self) -> u32 {
        self.inner.initial_budget.remaining
    }

    fn __repr__(&self) -> String {
        format!(
            "HarnessSpec(id={:?}, version={:?}, tokens={})",
            self.inner.id.as_str(),
            self.inner.version.to_string(),
            self.inner.initial_budget.remaining,
        )
    }
}

// ----- LoopStrategy + TerminationStrategy handles ------------------------

#[pyclass(name = "LoopStrategy", module = "atomr_agents._native.harness")]
#[derive(Clone)]
pub struct PyLoopStrategy {
    pub(crate) inner: Arc<dyn LoopStrategy>,
}

#[pymethods]
impl PyLoopStrategy {
    fn __repr__(&self) -> String {
        "LoopStrategy(handle)".into()
    }
}

#[pyclass(name = "TerminationStrategy", module = "atomr_agents._native.harness")]
#[derive(Clone)]
pub struct PyTerminationStrategy {
    pub(crate) inner: Arc<dyn TerminationStrategy>,
}

#[pymethods]
impl PyTerminationStrategy {
    fn __repr__(&self) -> String {
        "TerminationStrategy(handle)".into()
    }
}

// ----- Termination factories ---------------------------------------------

#[pyclass(name = "IterationCapTermination", module = "atomr_agents._native.harness")]
pub struct PyIterationCapTermination {
    pub(crate) inner: IterationCapTermination,
}

#[pymethods]
impl PyIterationCapTermination {
    #[new]
    fn new(cap: u64) -> Self {
        Self {
            inner: IterationCapTermination { cap },
        }
    }

    #[getter]
    fn cap(&self) -> u64 {
        self.inner.cap
    }

    fn __repr__(&self) -> String {
        format!("IterationCapTermination(cap={})", self.inner.cap)
    }
}

#[pyfunction]
fn iteration_cap(cap: u64) -> PyTerminationStrategy {
    PyTerminationStrategy {
        inner: Arc::new(IterationCapTermination { cap }),
    }
}

#[pyfunction]
fn termination_from_factory(key: String) -> PyResult<PyTerminationStrategy> {
    let target = crate::guest::must_lookup("strategy:termination", &key)?;
    Ok(PyTerminationStrategy {
        inner: Arc::new(PyTerminationStrategyAdapter { target }),
    })
}

pub(crate) struct PyTerminationStrategyAdapter {
    target: Arc<PyObject>,
}

impl TerminationStrategy for PyTerminationStrategyAdapter {
    fn should_terminate(&self, state: &HarnessState) -> Termination {
        let target = self.target.clone();
        Python::with_gil(|py| -> PyResult<Termination> {
            let bound = target.bind(py);
            let st = PyDict::new_bound(py);
            st.set_item("iteration", state.iteration)?;
            st.set_item("remaining_tokens", state.remaining_tokens)?;
            let instance: Bound<'_, PyAny> = if bound.hasattr("should_terminate")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("should_terminate")?.call1((st,))?;
            // Accept either a Termination instance or a truthy value
            // (string = reason, True = generic done). The harness
            // Termination has `Continue | Done(&'static str)` so any
            // dynamic reason gets `Box::leak`ed.
            if let Ok(t) = r.extract::<crate::strategy::PyTermination>() {
                return Ok(match t.kind.as_str() {
                    "done" => {
                        let s = t.reason.unwrap_or_else(|| "done".into());
                        Termination::Done(Box::leak(s.into_boxed_str()))
                    }
                    _ => Termination::Continue,
                });
            }
            if r.is_truthy()? {
                let reason: String = r.extract::<String>().unwrap_or_else(|_| "done".to_string());
                Ok(Termination::Done(Box::leak(reason.into_boxed_str())))
            } else {
                Ok(Termination::Continue)
            }
        })
        .unwrap_or(Termination::Continue)
    }
}

// ----- LoopStrategy from callable / from Python guest --------------------

/// Build a `LoopStrategy` from a `Callable`. The callable takes the
/// current `working_memory` value and returns either a continued value
/// or a "done" envelope `{"done": output}`. Continuing returns whatever
/// value the callable produced.
#[pyfunction]
fn loop_strategy_from_callable(callable: PyCallable) -> PyLoopStrategy {
    struct CallableLoop(CallableHandle);
    #[async_trait]
    impl LoopStrategy for CallableLoop {
        async fn step(&self, state: &mut HarnessState) -> AgentResult<StepOutcome> {
            let ctx = CallCtx {
                agent_id: None,
                tokens: TokenBudget::new(state.remaining_tokens),
                time: atomr_agents_core::TimeBudget::new(std::time::Duration::from_secs(3600)),
                money: atomr_agents_core::MoneyBudget::from_usd(1_000.0),
                iterations: atomr_agents_core::IterationBudget::new(state.iteration as u32 + 1),
                trace: Vec::new(),
            };
            let input = state.working_memory.clone();
            let out = self.0.call(input, ctx).await?;
            // Inspect the result for a "done" envelope.
            match out {
                Value::Object(ref m) if m.contains_key("done") => Ok(StepOutcome::Done {
                    output: m.get("done").cloned().unwrap_or(Value::Null),
                    label: "callable_done".into(),
                }),
                v => Ok(StepOutcome::Continue {
                    working_memory: v,
                    label: "callable_step".into(),
                }),
            }
        }
    }
    PyLoopStrategy {
        inner: Arc::new(CallableLoop(callable.inner)),
    }
}

#[pyfunction]
fn loop_strategy_from_factory(key: String) -> PyResult<PyLoopStrategy> {
    let target = crate::guest::must_lookup("strategy:loop", &key)?;
    Ok(PyLoopStrategy {
        inner: Arc::new(PyLoopStrategyAdapter { target }),
    })
}

pub(crate) struct PyLoopStrategyAdapter {
    target: Arc<PyObject>,
}

#[async_trait]
impl LoopStrategy for PyLoopStrategyAdapter {
    async fn step(&self, state: &mut HarnessState) -> AgentResult<StepOutcome> {
        let target = self.target.clone();
        let iter = state.iteration;
        let remaining = state.remaining_tokens;
        let working = state.working_memory.clone();
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let st = PyDict::new_bound(py);
            st.set_item("iteration", iter)?;
            st.set_item("remaining_tokens", remaining)?;
            st.set_item("working_memory", json_to_py(py, &working)?)?;
            let instance: Bound<'_, PyAny> = if bound.hasattr("step")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("step")?.call1((st,))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("py loop step: {e}")))?;
        let final_val = await_if_coro(coro_or_val).await?;
        Python::with_gil(|py| -> PyResult<StepOutcome> {
            let bound = final_val.bind(py);
            let v = py_to_json(py, bound)?;
            // Two shapes: {"done": value, "label"?: str} or {"working_memory": value, "label"?: str}.
            if let Value::Object(m) = &v {
                let label = m
                    .get("label")
                    .and_then(|v| v.as_str())
                    .unwrap_or("step")
                    .to_string();
                if let Some(done) = m.get("done") {
                    return Ok(StepOutcome::Done {
                        output: done.clone(),
                        label,
                    });
                }
                if let Some(wm) = m.get("working_memory") {
                    return Ok(StepOutcome::Continue {
                        working_memory: wm.clone(),
                        label,
                    });
                }
            }
            // Fallback: treat the entire value as continue/working_memory.
            Ok(StepOutcome::Continue {
                working_memory: v,
                label: "step".into(),
            })
        })
        .map_err(|e| AgentError::Internal(format!("py loop step result: {e}")))
    }
}

// ----- HarnessState + StepEvent ------------------------------------------

#[pyclass(name = "HarnessState", module = "atomr_agents._native.harness")]
#[derive(Clone)]
pub struct PyHarnessState {
    pub(crate) iteration: u64,
    pub(crate) remaining_tokens: u32,
    pub(crate) working_memory: Value,
    pub(crate) history: Vec<StepEvent>,
}

#[pymethods]
impl PyHarnessState {
    #[getter]
    fn iteration(&self) -> u64 {
        self.iteration
    }
    #[getter]
    fn remaining_tokens(&self) -> u32 {
        self.remaining_tokens
    }
    #[getter]
    fn working_memory(&self, py: Python<'_>) -> PyResult<PyObject> {
        json_to_py(py, &self.working_memory)
    }
    #[getter]
    fn history(&self) -> Vec<PyStepEvent> {
        self.history
            .iter()
            .map(|e| PyStepEvent {
                iteration: e.iteration,
                outcome: e.outcome.clone(),
                timestamp_ms: e.timestamp_ms,
            })
            .collect()
    }
    fn __repr__(&self) -> String {
        format!(
            "HarnessState(iter={}, tokens={}, history={})",
            self.iteration,
            self.remaining_tokens,
            self.history.len()
        )
    }
}

impl From<&HarnessState> for PyHarnessState {
    fn from(s: &HarnessState) -> Self {
        Self {
            iteration: s.iteration,
            remaining_tokens: s.remaining_tokens,
            working_memory: s.working_memory.clone(),
            history: s.history.clone(),
        }
    }
}

#[pyclass(name = "StepEvent", module = "atomr_agents._native.harness")]
#[derive(Clone)]
pub struct PyStepEvent {
    #[pyo3(get)]
    pub iteration: u64,
    #[pyo3(get)]
    pub outcome: String,
    #[pyo3(get)]
    pub timestamp_ms: i64,
}

#[pymethods]
impl PyStepEvent {
    fn __repr__(&self) -> String {
        format!(
            "StepEvent(iter={}, outcome={:?}, ts={})",
            self.iteration, self.outcome, self.timestamp_ms
        )
    }
}

// ----- Harness + runner ---------------------------------------------------

#[pyclass(name = "Harness", module = "atomr_agents._native.harness")]
pub struct PyHarness {
    pub(crate) inner: Arc<Harness<Box<dyn LoopStrategy>, Box<dyn TerminationStrategy>>>,
}

#[pymethods]
impl PyHarness {
    #[new]
    #[pyo3(signature = (spec, loop_strategy, termination, bus=None))]
    fn new(
        spec: PyHarnessSpec,
        loop_strategy: PyLoopStrategy,
        termination: PyTerminationStrategy,
        bus: Option<crate::observability::PyEventBus>,
    ) -> Self {
        // Wrap Arcs in Box<dyn> via small forwarder structs.
        struct ArcLoop(Arc<dyn LoopStrategy>);
        #[async_trait]
        impl LoopStrategy for ArcLoop {
            async fn step(&self, state: &mut HarnessState) -> AgentResult<StepOutcome> {
                self.0.step(state).await
            }
        }
        struct ArcTerm(Arc<dyn TerminationStrategy>);
        impl TerminationStrategy for ArcTerm {
            fn should_terminate(&self, state: &HarnessState) -> Termination {
                self.0.should_terminate(state)
            }
        }
        let h = Harness {
            spec: spec.inner,
            loop_strategy: Box::new(ArcLoop(loop_strategy.inner)) as Box<dyn LoopStrategy>,
            termination: Box::new(ArcTerm(termination.inner)) as Box<dyn TerminationStrategy>,
            bus: bus.map(|b| b.inner).unwrap_or_else(EventBus::new),
        };
        Self { inner: Arc::new(h) }
    }

    /// Run the harness loop to completion. Returns the final value
    /// returned by `StepOutcome::Done`, or the working_memory at the
    /// time the termination strategy fired.
    fn run<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let out = inner.run().await.map_err(crate::errors::map)?;
            Python::with_gil(|py| json_to_py(py, &out))
        })
    }

    /// Project this harness as a `Callable` that ignores its input and
    /// returns the final value.
    fn as_callable(&self) -> PyCallable {
        let inner = self.inner.clone();
        let h: CallableHandle = Arc::new(atomr_agents_callable::FnCallable::labeled(
            "harness",
            move |_v: Value, _ctx| {
                let inner = inner.clone();
                async move { inner.run().await }
            },
        ));
        PyCallable::from_handle(h)
    }

    fn __repr__(&self) -> String {
        format!("Harness(id={:?})", self.inner.spec.id.as_str())
    }
}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "harness")?;
    m.add_class::<PyHarnessSpec>()?;
    m.add_class::<PyIterationCapTermination>()?;
    m.add_class::<PyLoopStrategy>()?;
    m.add_class::<PyTerminationStrategy>()?;
    m.add_class::<PyHarnessState>()?;
    m.add_class::<PyStepEvent>()?;
    m.add_class::<PyHarness>()?;
    m.add_function(wrap_pyfunction!(iteration_cap, &m)?)?;
    m.add_function(wrap_pyfunction!(termination_from_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(loop_strategy_from_callable, &m)?)?;
    m.add_function(wrap_pyfunction!(loop_strategy_from_factory, &m)?)?;
    parent.add_submodule(&m)?;
    Ok(())
}
