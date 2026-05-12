//! Structured observability surface: `Event`, `EventEnvelope`,
//! `EventBus`, an async `EventStream` iterator, and the `Tracer`
//! sinks (stdout, JSONL, LangSmith).
//!
//! `EventBus.subscribe(callback)` keeps the existing sync-callback
//! pattern that tests rely on. `EventBus.stream()` returns an
//! `EventStream` that can be `async for`'d — the Python parity-wave
//! analogue of atomr-infer's `TokenStream`.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{
    AgentError, AgentId, Event, EventEnvelope, Result as AgentResult, RunId, ToolId, WorkflowId,
};
use atomr_agents_observability::EventBus;
use parking_lot::Mutex;
use pyo3::exceptions::{PyRuntimeError, PyStopAsyncIteration};
use pyo3::prelude::*;
use tokio::sync::mpsc;

use crate::conv::json_to_py;
use crate::strategy::await_if_coro;

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
#[derive(Clone)]
pub struct PyEventBus {
    pub(crate) inner: EventBus,
}

impl PyEventBus {
    pub(crate) fn new_default() -> Self {
        Self {
            inner: EventBus::new(),
        }
    }
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

    /// Attach a tracer to this bus. The tracer's `on_event` is invoked
    /// for every subsequent emitted event, and the tracer's
    /// `RunTreeBuilder` (when present) is auto-attached so the run
    /// tree is fed by the same bus. Call `await tracer.flush()` from
    /// Python when the run is complete to drain accumulated nodes to
    /// the tracer's sink.
    fn attach_tracer(&self, tracer: &PyTracer) {
        if let Some(builder) = tracer.builder.clone() {
            builder.attach(&self.inner);
        }
        let inner = tracer.inner.clone();
        self.inner.subscribe(move |env: &EventEnvelope| {
            let tracer = inner.clone();
            let env = env.clone();
            // The `Tracer::on_event` future is `Send`, so spawn it on
            // the current tokio runtime when one is available.
            // Otherwise fall back to a one-shot current-thread runtime
            // so sync callers (tests) still see `on_event` fire.
            match tokio::runtime::Handle::try_current() {
                Ok(handle) => {
                    handle.spawn(async move {
                        let _ = tracer.on_event(&env).await;
                    });
                }
                Err(_) => {
                    if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        rt.block_on(async move {
                            let _ = tracer.on_event(&env).await;
                        });
                    }
                }
            }
        });
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
    StdoutTracer as RustStdoutTracer, Tracer, TracerSink,
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

// ----- PyTracer dyn handle --------------------------------------------------
//
// `Tracer` is `Send + Sync + 'static` in the Rust crate, so we hold it
// behind an `Arc`. Stock tracers are produced by the factories below.
// Python-defined tracers register through
// `guest.register_tracer_factory(key, target)` and are materialised
// via `tracer_from_factory(key)`, which wraps the Python target in
// `PyTracerAdapter`.
//
// Limitations:
//   * `jsonl_tracer(path)` creates a fresh `RunTreeBuilder` per tracer
//     and writes via a local `FileLineSink`. Callers that want to
//     share the builder with their own `RunTreeBuilder` should use
//     `RunTreeBuilder.flush_jsonl()` instead.
//   * `lang_smith_tracer(api_key, project)` currently ignores
//     `api_key` — the upstream Rust `LangSmithTracer` writes to a
//     generic `TracerSink` and no HTTP sink is shipped yet, so this
//     factory uses an in-memory sink suitable for offline import or
//     mock-server integration tests. For real LangSmith ingestion,
//     register a Python tracer via `guest.register_tracer_factory`.

/// Append-one-line-per-emit file sink. The observability crate's own
/// `FileSink` is not re-exported, so we ship a local copy with the
/// same semantics.
struct FileLineSink {
    path: PathBuf,
}

impl FileLineSink {
    fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[async_trait]
impl TracerSink for FileLineSink {
    async fn emit(&self, payload: &str) -> AgentResult<()> {
        use tokio::io::AsyncWriteExt;
        let mut f = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await
            .map_err(|e| AgentError::Internal(format!("file sink open: {e}")))?;
        f.write_all(payload.as_bytes())
            .await
            .map_err(|e| AgentError::Internal(format!("file sink write: {e}")))?;
        f.write_all(b"\n")
            .await
            .map_err(|e| AgentError::Internal(format!("file sink newline: {e}")))?;
        Ok(())
    }
}

/// Python-facing `Tracer` handle. Wraps an `Arc<dyn Tracer>` plus the
/// optional `RunTreeBuilder` it was constructed against so
/// `PyEventBus::attach_tracer` can auto-attach the builder to the bus.
#[pyclass(name = "Tracer", module = "atomr_agents._native.observability")]
#[derive(Clone)]
pub struct PyTracer {
    pub(crate) inner: Arc<dyn Tracer>,
    pub(crate) builder: Option<Arc<RunTreeBuilder>>,
}

#[pymethods]
impl PyTracer {
    /// Forward an `Event` through this tracer's `on_event` hook.
    /// Mirrors the Rust trait method directly. Async.
    fn on_event<'py>(&self, py: Python<'py>, event: &PyEvent) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let env = event.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner.on_event(&env).await.map_err(crate::errors::map)?;
            Ok(())
        })
    }

    /// Flush accumulated runs through this tracer's sink. Async.
    fn flush<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner.flush().await.map_err(crate::errors::map)?;
            Ok(())
        })
    }

    fn __repr__(&self) -> String {
        "Tracer(handle)".to_string()
    }
}

// ----- PyTracerAdapter ------------------------------------------------------
//
// Adapts a Python object implementing `on_event` / `flush` (sync or
// async) into a Rust `Tracer`. The target is looked up via
// `crate::guest::must_lookup("tracer", &key)`.

pub(crate) struct PyTracerAdapter {
    pub(crate) target: Arc<PyObject>,
}

#[async_trait]
impl Tracer for PyTracerAdapter {
    async fn on_event(&self, env: &EventEnvelope) -> AgentResult<()> {
        let target = self.target.clone();
        let env_clone = env.clone();
        let maybe_call = Python::with_gil(|py| -> PyResult<Option<PyObject>> {
            let bound = target.bind(py);
            let instance: Bound<'_, PyAny> = if bound.hasattr("on_event")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            if !instance.hasattr("on_event")? {
                // Guest tracer opts out of per-event hooks: no-op.
                return Ok(None);
            }
            let py_event = Py::new(py, PyEvent { inner: env_clone })?;
            let r = instance.getattr("on_event")?.call1((py_event,))?;
            Ok(Some(r.unbind()))
        })
        .map_err(|e| AgentError::Internal(format!("py tracer on_event: {e}")))?;
        if let Some(value) = maybe_call {
            let _ = await_if_coro(value).await?;
        }
        Ok(())
    }

    async fn flush(&self) -> AgentResult<()> {
        let target = self.target.clone();
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let instance: Bound<'_, PyAny> = if bound.hasattr("flush")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("flush")?.call0()?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("py tracer flush: {e}")))?;
        let _ = await_if_coro(coro_or_val).await?;
        Ok(())
    }
}

// ----- Factory functions ----------------------------------------------------

/// Build a JSONL tracer that appends one line per run node to `path`.
/// The tracer owns a fresh `RunTreeBuilder`; attach it to a bus with
/// `EventBus.attach_tracer(tracer)` and call `await tracer.flush()`
/// once the run is done.
#[pyfunction]
fn jsonl_tracer(path: String) -> PyTracer {
    let builder = Arc::new(RunTreeBuilder::new());
    let sink: Arc<dyn TracerSink> = Arc::new(FileLineSink::new(path));
    let tracer = RustJsonlTracer::new(builder.clone(), sink);
    PyTracer {
        inner: Arc::new(tracer),
        builder: Some(builder),
    }
}

/// Build a LangSmith-shaped tracer. `api_key` is accepted for API
/// parity but currently unused — the upstream Rust tracer writes
/// through a `TracerSink` and no HTTP transport is shipped yet, so
/// this binding stages records into an in-memory sink. For real
/// ingestion, register a Python tracer via
/// `guest.register_tracer_factory`.
#[pyfunction]
fn lang_smith_tracer(_api_key: String, project: String) -> PyTracer {
    let builder = Arc::new(RunTreeBuilder::new());
    let (tracer, _sink) = RustLangSmithTracer::in_memory(builder.clone(), project);
    PyTracer {
        inner: Arc::new(tracer),
        builder: Some(builder),
    }
}

/// Build a stdout tracer that pretty-prints the accumulated run tree
/// on `await tracer.flush()`.
#[pyfunction]
fn stdout_tracer() -> PyTracer {
    let builder = Arc::new(RunTreeBuilder::new());
    let tracer = RustStdoutTracer::new(builder.clone());
    PyTracer {
        inner: Arc::new(tracer),
        builder: Some(builder),
    }
}

/// Materialise a Python-registered tracer
/// (`guest.register_tracer_factory(key, target)`). The adapter calls
/// back into the target's `on_event` / `flush` methods; either may be
/// sync or async.
#[pyfunction]
fn tracer_from_factory(key: String) -> PyResult<PyTracer> {
    let target = crate::guest::must_lookup("tracer", &key)?;
    Ok(PyTracer {
        inner: Arc::new(PyTracerAdapter { target }),
        builder: None,
    })
}

// ----- Module registration --------------------------------------------------

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "observability")?;
    m.add_class::<PyEvent>()?;
    m.add_class::<PyEventBus>()?;
    m.add_class::<PyEventStream>()?;
    m.add_class::<PyRunTreeBuilder>()?;
    m.add_class::<PyTracer>()?;
    m.add_function(wrap_pyfunction!(jsonl_tracer, &m)?)?;
    m.add_function(wrap_pyfunction!(lang_smith_tracer, &m)?)?;
    m.add_function(wrap_pyfunction!(stdout_tracer, &m)?)?;
    m.add_function(wrap_pyfunction!(tracer_from_factory, &m)?)?;
    parent.add_submodule(&m)?;
    Ok(())
}
