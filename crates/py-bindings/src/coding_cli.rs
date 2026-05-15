//! Python bindings for the coding-cli harness.
//!
//! Exposes `atomr_agents._native.coding_cli`:
//!
//! - `CodingCliHarness` — `local_default()` builder + `run_headless()`
//!   (async, returns the result dict), `start_interactive()` (async,
//!   returns a session), `events()` for the normalized SSE-shaped
//!   stream.
//! - `CodingCliEventStream` — `recv()` async iterator over events.
//! - `InteractiveSession` — `send_keys`, `resize`, `read`, `stop`.

use std::sync::Arc;

use atomr_agents_coding_cli_core::{CliRequest, CliResult, CliSessionId, CodingCliEventStream};
use atomr_agents_coding_cli_harness::{
    CodingCliHarness, InteractiveSessionHandle, SessionEvent,
};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use tokio::sync::{broadcast, Mutex as AsyncMutex};

use crate::conv::{json_to_py, py_to_json};

// ----- helpers -----------------------------------------------------------

fn req_from_pydict(py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<CliRequest> {
    let value = py_to_json(py, obj)?;
    serde_json::from_value::<CliRequest>(value)
        .map_err(|e| PyValueError::new_err(format!("invalid CliRequest: {e}")))
}

fn result_to_py(py: Python<'_>, r: &CliResult) -> PyResult<PyObject> {
    let value = serde_json::to_value(r).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    json_to_py(py, &value)
}

// ----- CodingCliEventStream ----------------------------------------------

#[pyclass(name = "CodingCliEventStream", module = "atomr_agents._native.coding_cli")]
pub struct PyCodingCliEventStream {
    inner: Arc<AsyncMutex<CodingCliEventStream>>,
}

#[pymethods]
impl PyCodingCliEventStream {
    /// Async `recv()` → dict, or `None` once the stream closes.
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

// ----- InteractiveSession ------------------------------------------------

#[pyclass(name = "InteractiveSession", module = "atomr_agents._native.coding_cli")]
pub struct PyInteractiveSession {
    handle: Arc<InteractiveSessionHandle>,
    rx: Arc<AsyncMutex<broadcast::Receiver<SessionEvent>>>,
    harness: Arc<CodingCliHarness>,
}

#[pymethods]
impl PyInteractiveSession {
    #[getter]
    fn id(&self) -> String {
        self.handle.id.to_string()
    }

    #[getter]
    fn vendor(&self) -> String {
        self.handle.vendor.as_str().to_string()
    }

    #[getter]
    fn tmux_session(&self) -> String {
        self.handle.tmux_session.clone()
    }

    /// Send bytes (typed keystrokes / paste) to the tmux-wrapped CLI.
    fn send_keys<'py>(&self, py: Python<'py>, data: Vec<u8>) -> PyResult<Bound<'py, PyAny>> {
        let h = self.handle.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let ok = h.send_stdin(data).await;
            Ok(ok)
        })
    }

    /// Resize the PTY window.
    fn resize<'py>(&self, py: Python<'py>, cols: u16, rows: u16) -> PyResult<Bound<'py, PyAny>> {
        let h = self.handle.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let ok = h.resize(cols, rows).await;
            Ok(ok)
        })
    }

    /// Await the next PTY chunk as `bytes`, or `None` once exited.
    fn read<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let rx = self.rx.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let event = {
                let mut guard = rx.lock().await;
                loop {
                    match guard.recv().await {
                        Ok(ev) => break Some(ev),
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => break None,
                    }
                }
            };
            Python::with_gil(|py| match event {
                Some(SessionEvent::Bytes(b)) => Ok(PyBytes::new_bound(py, &b).into_py(py)),
                Some(SessionEvent::Exited { .. }) | None => Ok(py.None()),
            })
        })
    }

    /// Tear down the tmux session and remove it from the registry.
    fn stop<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let id = self.handle.id.clone();
        let harness = self.harness.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            harness
                .stop_interactive(&id)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            Ok(())
        })
    }

    fn __repr__(&self) -> String {
        format!(
            "InteractiveSession(id={}, vendor={})",
            self.handle.id, self.handle.vendor
        )
    }
}

// ----- CodingCliHarness --------------------------------------------------

#[pyclass(name = "CodingCliHarness", module = "atomr_agents._native.coding_cli")]
pub struct PyCodingCliHarness {
    inner: Arc<CodingCliHarness>,
}

#[pymethods]
impl PyCodingCliHarness {
    /// Build a harness with the in-memory store, default vendors, and
    /// the local isolator. Use this for headless host execution.
    #[staticmethod]
    fn local_default() -> Self {
        Self {
            inner: Arc::new(CodingCliHarness::local_default()),
        }
    }

    /// List the wired-up vendor kinds as strings.
    fn vendors(&self) -> Vec<String> {
        self.inner
            .available_vendors()
            .into_iter()
            .map(|k| k.as_str().to_string())
            .collect()
    }

    /// Subscribe to the harness's normalized event stream.
    fn events(&self) -> PyCodingCliEventStream {
        PyCodingCliEventStream {
            inner: Arc::new(AsyncMutex::new(self.inner.events())),
        }
    }

    /// Async: run a headless request and resolve to a result dict.
    fn run_headless<'py>(
        &self,
        py: Python<'py>,
        request: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let req = req_from_pydict(py, request)?;
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let result = inner.run(req).await.map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            Python::with_gil(|py| result_to_py(py, &result))
        })
    }

    /// Async: kick off an interactive session, returning an
    /// `InteractiveSession` handle.
    fn start_interactive<'py>(
        &self,
        py: Python<'py>,
        request: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let req = req_from_pydict(py, request)?;
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let handle = inner
                .start_interactive(req)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            let rx = handle.subscribe();
            Python::with_gil(|py| {
                Py::new(
                    py,
                    PyInteractiveSession {
                        handle,
                        rx: Arc::new(AsyncMutex::new(rx)),
                        harness: inner,
                    },
                )
            })
        })
    }

    /// List currently-active interactive sessions as dicts.
    fn sessions<'py>(&self, py: Python<'py>) -> PyResult<PyObject> {
        let list = self.inner.sessions().list();
        let arr = pyo3::types::PyList::empty_bound(py);
        for h in list {
            let d = PyDict::new_bound(py);
            d.set_item("id", h.id.to_string())?;
            d.set_item("vendor", h.vendor.as_str())?;
            d.set_item("tmux_session", h.tmux_session.clone())?;
            d.set_item("started_at", h.started_at.to_rfc3339())?;
            arr.append(d)?;
        }
        Ok(arr.into())
    }

    /// Stop an interactive session by id.
    fn stop_session<'py>(&self, py: Python<'py>, id: String) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner
                .stop_interactive(&CliSessionId::from(id))
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            Ok(())
        })
    }

    /// Cooperative cancel for any in-flight headless run.
    fn cancel(&self) {
        self.inner.cancel();
    }

    fn __repr__(&self) -> String {
        format!(
            "CodingCliHarness(vendors={:?})",
            self.inner
                .available_vendors()
                .into_iter()
                .map(|k| k.as_str().to_string())
                .collect::<Vec<_>>(),
        )
    }
}

// ----- module registration -----------------------------------------------

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "coding_cli")?;
    m.add_class::<PyCodingCliHarness>()?;
    m.add_class::<PyCodingCliEventStream>()?;
    m.add_class::<PyInteractiveSession>()?;
    parent.add_submodule(&m)?;
    Ok(())
}
