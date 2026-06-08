//! Python bindings for the Claude Agent SDK harness.
//!
//! Exposes `atomr_agents._native.agent_sdk`:
//!
//! - `AgentSdkHarness` — `from_python_backend(key, spec)` / `mock()`
//!   builders, async `run()` (final result dict), `session()` (interactive
//!   bidirectional session = the actor surface), `events()` (event
//!   stream), `sessions()`.
//! - `AgentSdkSession` — `query`, `interrupt`, `set_permission_mode`,
//!   `set_model`, `events`, `close`.
//! - `AgentSdkEventStream` — `__aiter__` / `__anext__` async iterator.
//! - `invoke_tool` — run a registered atomr `@tool` as an in-process tool.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use pyo3::exceptions::{PyStopAsyncIteration, PyValueError};
use pyo3::prelude::*;
use tokio::sync::Mutex as AsyncMutex;

use atomr_agents_agent_sdk_core::{
    AgentSdkEvent, AgentSdkEventStream, AgentSessionId, QueryRequest, SessionSpec,
};
use atomr_agents_agent_sdk_harness::{AgentSdkHarness, AgentSdkHarnessSpec, InteractiveAgentSession};

use crate::conv::{json_to_py, py_to_json, py_to_value_or};

fn query_request_from_py(py: Python<'_>, request: &Bound<'_, PyAny>) -> PyResult<QueryRequest> {
    if let Ok(s) = request.extract::<String>() {
        return Ok(QueryRequest::new(s));
    }
    let value = py_to_json(py, request)?;
    serde_json::from_value::<QueryRequest>(value)
        .map_err(|e| PyValueError::new_err(format!("invalid request: {e}")))
}

// ----- AgentSdkEventStream ------------------------------------------------

#[pyclass(name = "AgentSdkEventStream", module = "atomr_agents._native.agent_sdk")]
pub struct PyAgentSdkEventStream {
    inner: Arc<AsyncMutex<AgentSdkEventStream>>,
    stop_on_finish: bool,
    done: Arc<AtomicBool>,
}

#[pymethods]
impl PyAgentSdkEventStream {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(slf: PyRef<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = slf.inner.clone();
        let stop_on_finish = slf.stop_on_finish;
        let done = slf.done.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            if done.load(Ordering::Relaxed) {
                return Err(PyStopAsyncIteration::new_err(""));
            }
            let next = {
                let mut guard = inner.lock().await;
                guard.recv().await
            };
            match next {
                None => Err(PyStopAsyncIteration::new_err("")),
                Some(ev) => {
                    if stop_on_finish && matches!(ev, AgentSdkEvent::RunFinished { .. }) {
                        done.store(true, Ordering::Relaxed);
                    }
                    let value = serde_json::to_value(&ev).unwrap_or(serde_json::Value::Null);
                    Python::with_gil(|py| json_to_py(py, &value))
                }
            }
        })
    }

    /// Explicit `recv()` → dict, or `None` once the stream ends.
    fn recv<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let next = {
                let mut guard = inner.lock().await;
                guard.recv().await
            };
            Python::with_gil(|py| match next {
                None => Ok(py.None()),
                Some(ev) => {
                    let value = serde_json::to_value(&ev).unwrap_or(serde_json::Value::Null);
                    json_to_py(py, &value)
                }
            })
        })
    }
}

// ----- AgentSdkSession ----------------------------------------------------

#[pyclass(name = "AgentSdkSession", module = "atomr_agents._native.agent_sdk")]
pub struct PyAgentSdkSession {
    session: Arc<InteractiveAgentSession>,
    harness: Arc<AgentSdkHarness>,
}

#[pymethods]
impl PyAgentSdkSession {
    #[getter]
    fn session_id(&self) -> String {
        self.session.id.to_string()
    }

    /// Send a prompt; the response streams onto `events()`.
    fn query<'py>(&self, py: Python<'py>, prompt: String) -> PyResult<Bound<'py, PyAny>> {
        let session = self.session.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            session.send(prompt).await.map_err(crate::errors::map)?;
            Ok(())
        })
    }

    /// Subscribe to the event stream.
    fn events(&self) -> PyAgentSdkEventStream {
        PyAgentSdkEventStream {
            inner: Arc::new(AsyncMutex::new(self.session.subscribe())),
            stop_on_finish: false,
            done: Arc::new(AtomicBool::new(false)),
        }
    }

    fn interrupt<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let session = self.session.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            session.interrupt().await.map_err(crate::errors::map)?;
            Ok(())
        })
    }

    fn set_permission_mode<'py>(&self, py: Python<'py>, mode: String) -> PyResult<Bound<'py, PyAny>> {
        let session = self.session.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            session.set_permission_mode(mode).await.map_err(crate::errors::map)?;
            Ok(())
        })
    }

    fn set_model<'py>(&self, py: Python<'py>, model: String) -> PyResult<Bound<'py, PyAny>> {
        let session = self.session.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            session.set_model(model).await.map_err(crate::errors::map)?;
            Ok(())
        })
    }

    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let harness = self.harness.clone();
        let id = self.session.id.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            harness.stop_session(&id).await.map_err(crate::errors::map)?;
            Ok(())
        })
    }

    fn __repr__(&self) -> String {
        format!("AgentSdkSession(id={})", self.session.id)
    }
}

// ----- AgentSdkHarness ----------------------------------------------------

#[pyclass(name = "AgentSdkHarness", module = "atomr_agents._native.agent_sdk")]
pub struct PyAgentSdkHarness {
    inner: Arc<AgentSdkHarness>,
}

#[pymethods]
impl PyAgentSdkHarness {
    /// Build a harness driving the registered Python `agent_sdk` backend.
    #[staticmethod]
    #[pyo3(signature = (backend_key, spec=None))]
    fn from_python_backend(
        py: Python<'_>,
        backend_key: String,
        spec: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let spec_val = py_to_value_or(py, spec, serde_json::json!({}))?;
        let spec: AgentSdkHarnessSpec =
            serde_json::from_value(spec_val).map_err(crate::errors::map)?;
        let backend = crate::guest::build_agent_sdk_backend(&backend_key)?;
        Ok(Self {
            inner: Arc::new(AgentSdkHarness::new(backend, spec)),
        })
    }

    /// Build a harness over the in-memory `MockBackend` (network-free).
    #[staticmethod]
    fn mock() -> Self {
        Self {
            inner: Arc::new(AgentSdkHarness::local_default()),
        }
    }

    #[getter]
    fn backend_name(&self) -> String {
        self.inner.backend_name().to_string()
    }

    fn live_count(&self) -> usize {
        self.inner.live_count()
    }

    /// Subscribe to the harness-wide event stream.
    fn events(&self) -> PyAgentSdkEventStream {
        PyAgentSdkEventStream {
            inner: Arc::new(AsyncMutex::new(self.inner.events())),
            stop_on_finish: false,
            done: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Async: run a headless query to completion, resolving to the result
    /// dict. `request` is a prompt string or a config dict with `prompt`.
    fn run<'py>(&self, py: Python<'py>, request: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let req = query_request_from_py(py, request)?;
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let result = inner.run(req).await.map_err(crate::errors::map)?;
            let value = serde_json::to_value(result).map_err(crate::errors::map)?;
            Python::with_gil(|py| json_to_py(py, &value))
        })
    }

    /// Async: open a stateful interactive session.
    #[pyo3(signature = (spec=None))]
    fn session<'py>(
        &self,
        py: Python<'py>,
        spec: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let spec_val = py_to_value_or(py, spec, serde_json::json!({}))?;
        let session_spec: SessionSpec =
            serde_json::from_value(spec_val).map_err(crate::errors::map)?;
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let session = inner.start_session(session_spec).await.map_err(crate::errors::map)?;
            Python::with_gil(|py| {
                Py::new(
                    py,
                    PyAgentSdkSession {
                        session,
                        harness: inner,
                    },
                )
            })
        })
    }

    /// List active interactive session ids.
    fn sessions(&self) -> Vec<String> {
        self.inner
            .sessions()
            .list()
            .iter()
            .map(|s| s.id.to_string())
            .collect()
    }

    /// Stop an interactive session by id.
    fn stop_session<'py>(&self, py: Python<'py>, id: String) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner
                .stop_session(&AgentSessionId::from(id))
                .await
                .map_err(crate::errors::map)?;
            Ok(())
        })
    }

    fn __repr__(&self) -> String {
        format!("AgentSdkHarness(backend={})", self.inner.backend_name())
    }
}

// ----- module registration -----------------------------------------------

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "agent_sdk")?;
    m.add_class::<PyAgentSdkHarness>()?;
    m.add_class::<PyAgentSdkSession>()?;
    m.add_class::<PyAgentSdkEventStream>()?;
    m.add_function(wrap_pyfunction!(crate::guest::invoke_tool, &m)?)?;
    parent.add_submodule(&m)?;
    Ok(())
}
