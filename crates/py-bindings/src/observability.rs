//! Structured observability surface: `Event`, `EventEnvelope`,
//! `EventBus`, an async `EventStream` iterator, and the `Tracer`
//! sinks (stdout, JSONL, LangSmith).
//!
//! `EventBus.subscribe(callback)` keeps the existing sync-callback
//! pattern that tests rely on. `EventBus.stream()` returns an
//! `EventStream` that can be `async for`'d — the Python parity-wave
//! analogue of atomr-infer's `TokenStream`.

use std::sync::Arc;

use atomr_agents_core::{AgentId, Event, EventEnvelope, RunId, ToolId, WorkflowId};
use atomr_agents_observability::EventBus;
use parking_lot::Mutex;
use pyo3::exceptions::{PyRuntimeError, PyStopAsyncIteration};
use pyo3::prelude::*;
use tokio::sync::mpsc;

use crate::conv::json_to_py;

// ----- PyEvent --------------------------------------------------------------

#[pyclass(name = "Event", module = "atomr_agents._native.observability")]
pub struct PyEvent {
    pub(crate) inner: EventEnvelope,
}

#[pymethods]
impl PyEvent {
    #[getter]
    fn timestamp_ms(&self) -> i64 {
        self.inner.timestamp_ms
    }

    /// Discriminator string for the event variant. Same names that
    /// the existing 0.2.x `EventBus` test_smoke.py asserts against.
    #[getter]
    fn kind(&self) -> &'static str {
        match self.inner.event {
            Event::StrategyResolved { .. } => "strategy_resolved",
            Event::ToolInvoked { .. } => "tool_invoked",
            Event::ToolCallStreamed { .. } => "tool_call_streamed",
            Event::AgentTurn { .. } => "agent_turn",
            Event::WorkflowStep { .. } => "workflow_step",
            Event::HarnessIteration { .. } => "harness_iteration",
            Event::Backpressure { .. } => "backpressure",
        }
    }

    #[getter]
    fn run_id(&self) -> Option<String> {
        self.inner.run_id.as_ref().map(|r| r.as_str().to_string())
    }

    #[getter]
    fn parent_run_id(&self) -> Option<String> {
        self.inner
            .parent_run_id
            .as_ref()
            .map(|r| r.as_str().to_string())
    }

    #[getter]
    fn tags(&self) -> Vec<String> {
        self.inner.tags.clone()
    }

    fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let v = serde_json::to_value(&self.inner)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        json_to_py(py, &v)
    }

    fn __repr__(&self) -> String {
        format!(
            "Event(kind={:?}, ts={}, run={:?})",
            self.kind(),
            self.inner.timestamp_ms,
            self.run_id()
        )
    }
}

// ----- PyEventBus -----------------------------------------------------------

#[pyclass(name = "EventBus", module = "atomr_agents._native.observability")]
pub struct PyEventBus {
    pub(crate) inner: EventBus,
}

#[pymethods]
impl PyEventBus {
    #[new]
    fn new() -> Self {
        Self {
            inner: EventBus::new(),
        }
    }

    /// Register a Python callable that receives each emitted event as
    /// a `PyEvent` dict-like wrapper.
    fn subscribe(&self, callback: PyObject) -> PyResult<()> {
        let cb = Arc::new(callback);
        self.inner.subscribe(move |env: &EventEnvelope| {
            let cb = cb.clone();
            let env = env.clone();
            Python::with_gil(|py| {
                if let Ok(pyev) =
                    Py::new(py, PyEvent { inner: env }).map(|e| e.into_py(py))
                {
                    let _ = cb.call1(py, (pyev,));
                }
            });
        });
        Ok(())
    }

    /// Open an async stream of events from this bus. Each emitted
    /// event after this call is queued and yielded via
    /// `__aiter__`/`__anext__`. Drop the stream to unsubscribe.
    fn stream(&self) -> PyEventStream {
        let (tx, rx) = mpsc::unbounded_channel::<EventEnvelope>();
        self.inner.subscribe(move |env: &EventEnvelope| {
            let _ = tx.send(env.clone());
        });
        PyEventStream {
            rx: Arc::new(tokio::sync::Mutex::new(rx)),
        }
    }

    fn emit_tool_invoked(
        &self,
        tool_id: String,
        args_hash: u64,
        elapsed_ms: u64,
        ok: bool,
    ) -> PyResult<()> {
        self.inner.emit(Event::ToolInvoked {
            tool_id: ToolId::from(tool_id),
            args_hash,
            elapsed_ms,
            ok,
        });
        Ok(())
    }

    #[pyo3(signature = (agent_id, input_tokens, output_tokens, elapsed_ms, reasoning_tokens=0, cached_tokens=0))]
    fn emit_agent_turn(
        &self,
        agent_id: String,
        input_tokens: u32,
        output_tokens: u32,
        elapsed_ms: u64,
        reasoning_tokens: u32,
        cached_tokens: u32,
    ) -> PyResult<()> {
        self.inner.emit(Event::AgentTurn {
            agent_id: AgentId::from(agent_id),
            input_tokens,
            output_tokens,
            reasoning_tokens,
            cached_tokens,
            finish_reason: None,
            elapsed_ms,
        });
        Ok(())
    }

    fn emit_workflow_step(
        &self,
        workflow_id: String,
        step_id: String,
        step_kind: String,
        elapsed_ms: u64,
        ok: bool,
    ) -> PyResult<()> {
        self.inner.emit(Event::WorkflowStep {
            workflow_id: WorkflowId::from(workflow_id),
            step_id,
            step_kind,
            elapsed_ms,
            ok,
        });
        Ok(())
    }

    #[pyo3(signature = (event_kind, run_id=None, parent_run_id=None))]
    fn emit_run(
        &self,
        event_kind: &str,
        run_id: Option<String>,
        parent_run_id: Option<String>,
    ) -> PyResult<()> {
        // Minimal echo for tests — full event-dict construction lives
        // in the typed emit_* helpers above; this keeps the run-id
        // wiring path covered.
        let ev = match event_kind {
            "backpressure" => Event::Backpressure {
                actor_path: "/python".to_string(),
                queued: 0,
                dropped: 0,
            },
            other => {
                return Err(PyRuntimeError::new_err(format!(
                    "emit_run: use the typed helper for {other:?}"
                )));
            }
        };
        self.inner.emit_run(
            ev,
            RunId::from(run_id.unwrap_or_else(|| "run-anonymous".to_string())),
            parent_run_id.map(RunId::from),
        );
        Ok(())
    }
}

// ----- Async iterator -------------------------------------------------------

#[pyclass(name = "EventStream", module = "atomr_agents._native.observability")]
pub struct PyEventStream {
    rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<EventEnvelope>>>,
}

#[pymethods]
impl PyEventStream {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(slf: PyRef<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let rx = slf.rx.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = rx.lock().await;
            match guard.recv().await {
                Some(env) => Python::with_gil(|py| Py::new(py, PyEvent { inner: env })),
                None => Err(PyStopAsyncIteration::new_err("")),
            }
        })
    }
}

// ----- RunTreeBuilder + tracer flush ---------------------------------------
//
// The observability crate's tracer surface is `RunTreeBuilder` →
// `Tracer::flush()`. We expose `RunTreeBuilder` plus an async
// `flush()` method that writes the accumulated tree to JSONL or
// LangSmith via in-memory sinks. Stdout tracer is direct — no sink.

use atomr_agents_observability::{
    JsonlTracer as RustJsonlTracer, LangSmithTracer as RustLangSmithTracer, RunTreeBuilder,
    StdoutTracer as RustStdoutTracer, Tracer,
};

#[pyclass(name = "RunTreeBuilder", module = "atomr_agents._native.observability")]
pub struct PyRunTreeBuilder {
    pub(crate) inner: Arc<RunTreeBuilder>,
}

#[pymethods]
impl PyRunTreeBuilder {
    #[new]
    fn new() -> Self {
        Self {
            inner: Arc::new(RunTreeBuilder::new()),
        }
    }

    /// Subscribe this builder to an `EventBus` so it accumulates a
    /// run tree from every emitted event.
    fn attach(&self, bus: &PyEventBus) {
        self.inner.clone().attach(&bus.inner);
    }

    /// Flush accumulated runs to stdout in a tree shape. Async.
    fn flush_stdout<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let builder = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let tracer = RustStdoutTracer::new(builder);
            tracer.flush().await.map_err(crate::errors::map)?;
            Ok(())
        })
    }

    /// Flush accumulated runs as JSONL. Returns the lines as a list
    /// of strings (in-memory sink). Async.
    fn flush_jsonl<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let builder = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let (tracer, sink) = RustJsonlTracer::in_memory(builder);
            tracer.flush().await.map_err(crate::errors::map)?;
            let lines: Vec<String> = sink.lines.lock().clone();
            Ok(lines)
        })
    }

    /// Flush accumulated runs as LangSmith records. Returns the JSON
    /// lines (in-memory sink). Async.
    fn flush_langsmith<'py>(
        &self,
        py: Python<'py>,
        project: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let builder = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let (tracer, sink) = RustLangSmithTracer::in_memory(builder, project);
            tracer.flush().await.map_err(crate::errors::map)?;
            let lines: Vec<String> = sink.lines.lock().clone();
            Ok(lines)
        })
    }

    fn __repr__(&self) -> String {
        "RunTreeBuilder()".to_string()
    }
}

// silence unused-Mutex warning on older builds
#[allow(dead_code)]
fn _mutex_keepalive(_: Mutex<()>) {}

// ----- Module registration --------------------------------------------------

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "observability")?;
    m.add_class::<PyEvent>()?;
    m.add_class::<PyEventBus>()?;
    m.add_class::<PyEventStream>()?;
    m.add_class::<PyRunTreeBuilder>()?;
    parent.add_submodule(&m)?;
    Ok(())
}
