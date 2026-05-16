//! Python bindings for the channel layer.
//!
//! Exposes `atomr_agents._native.channel`:
//!
//! - `ChannelHarness` — orchestrator with the in-memory store; attach
//!   the in-memory provider (real providers are configured server-side
//!   for now), open threads bound to any [`PyCallable`], and stream
//!   events.
//! - `ChannelEventStream` — async iterator over `ChannelEvent` dicts.
//! - `InMemoryProvider` — attach as a channel; tests push inbound via
//!   `inbox.push(...)`.
//! - `MessageContent` — convenience constructors mirroring the Rust enum.
//! - Helpers: `verify_webhook(provider, headers, body)`,
//!   `parse_webhook(provider, headers, body)` — synchronous wrappers
//!   for Python tests that fake an HTTP receiver.

use std::sync::Arc;

use atomr_agents_channel_core::memory::InMemoryProvider;
use atomr_agents_channel_core::{
    CallableHandle, ChannelEvent, ChannelEventStream, ChannelId, ChannelSpec, InboundMessage,
    MessageContent, PeerId, ProviderKind, ThreadId, ThreadRef, ThreadTarget,
};
use atomr_agents_channel_harness::ChannelHarness;
use atomr_agents_core::Value;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use tokio::sync::Mutex as AsyncMutex;

use crate::callable::PyCallable;
use crate::conv::json_to_py;

fn json_err(e: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(e.to_string())
}

fn rt_err(e: impl std::fmt::Display) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}

// ----- MessageContent -----------------------------------------------------

#[pyclass(name = "MessageContent", module = "atomr_agents._native.channel")]
#[derive(Clone)]
pub struct PyMessageContent {
    pub(crate) inner: MessageContent,
}

#[pymethods]
impl PyMessageContent {
    #[staticmethod]
    fn text(text: String) -> Self {
        Self {
            inner: MessageContent::text(text),
        }
    }

    #[staticmethod]
    #[pyo3(signature = (media_ref, mime, caption=None))]
    fn attachment(media_ref: String, mime: String, caption: Option<String>) -> Self {
        Self {
            inner: MessageContent::Attachment {
                media_ref,
                mime,
                caption,
            },
        }
    }

    #[getter]
    fn kind(&self) -> &'static str {
        match &self.inner {
            MessageContent::Text { .. } => "text",
            MessageContent::Attachment { .. } => "attachment",
            MessageContent::Mixed { .. } => "mixed",
        }
    }

    #[getter]
    fn as_text(&self) -> String {
        self.inner.as_text()
    }

    fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let value = serde_json::to_value(&self.inner).map_err(json_err)?;
        json_to_py(py, &value)
    }

    fn __repr__(&self) -> String {
        format!("MessageContent({:?})", self.inner)
    }
}

// ----- ChannelEventStream -------------------------------------------------

#[pyclass(name = "ChannelEventStream", module = "atomr_agents._native.channel")]
pub struct PyChannelEventStream {
    inner: Arc<AsyncMutex<ChannelEventStream>>,
}

#[pymethods]
impl PyChannelEventStream {
    fn recv<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let next: Option<ChannelEvent> = {
                let mut g = inner.lock().await;
                g.recv().await
            };
            Python::with_gil(|py| match next {
                None => Ok(py.None()),
                Some(ev) => {
                    let value = serde_json::to_value(&ev).unwrap_or(Value::Null);
                    json_to_py(py, &value)
                }
            })
        })
    }
}

// ----- InMemoryProvider + inbox ------------------------------------------

#[pyclass(name = "InMemoryProvider", module = "atomr_agents._native.channel")]
pub struct PyInMemoryProvider {
    pub(crate) inner: Arc<InMemoryProvider>,
    pub(crate) channel_id: ChannelId,
}

#[pymethods]
impl PyInMemoryProvider {
    #[new]
    fn new(channel_id: String) -> Self {
        let cid = ChannelId::from(channel_id);
        Self {
            inner: Arc::new(InMemoryProvider::new(cid.clone())),
            channel_id: cid,
        }
    }

    #[getter]
    fn channel_id(&self) -> String {
        self.channel_id.as_str().to_string()
    }

    /// Push an inbound message. `peer`, `provider_msg_id`, `text` are
    /// required; the harness fills in `thread_id` from `(channel, peer)`.
    #[pyo3(signature = (peer, provider_msg_id, text))]
    fn push_inbound(&self, peer: String, provider_msg_id: String, text: String) -> PyResult<()> {
        let peer = PeerId::from(peer);
        let thread_id = ThreadId::for_peer(&self.channel_id, &peer);
        let msg = InboundMessage {
            channel_id: self.channel_id.clone(),
            thread_id,
            peer,
            provider_msg_id,
            content: MessageContent::text(text),
            received_at: chrono::Utc::now(),
            raw: Value::Null,
        };
        self.inner.inbox().push(msg).map_err(rt_err)
    }
}

// ----- ThreadRef ----------------------------------------------------------

#[pyclass(name = "ThreadRef", module = "atomr_agents._native.channel")]
#[derive(Clone)]
pub struct PyThreadRef {
    pub(crate) inner: ThreadRef,
}

#[pymethods]
impl PyThreadRef {
    #[getter]
    fn id(&self) -> String {
        self.inner.id().as_str().to_string()
    }

    fn snapshot(&self, py: Python<'_>) -> PyResult<PyObject> {
        let snap = self.inner.snapshot();
        let d = PyDict::new_bound(py);
        d.set_item("id", snap.id.as_str())?;
        d.set_item("channel", snap.channel.as_str())?;
        d.set_item("peer", snap.peer.as_str())?;
        d.set_item("target_kind", snap.target.kind())?;
        d.set_item("target_label", snap.target.label())?;
        d.set_item("history_len", snap.history.len())?;
        Ok(d.into())
    }

    fn __repr__(&self) -> String {
        format!("ThreadRef(id={})", self.inner.id())
    }
}

// ----- ChannelHarness -----------------------------------------------------

#[pyclass(name = "ChannelHarness", module = "atomr_agents._native.channel")]
pub struct PyChannelHarness {
    inner: Arc<ChannelHarness>,
}

#[pymethods]
impl PyChannelHarness {
    #[new]
    fn new() -> Self {
        // Channel orchestrator spawns its inbound loop at construction
        // time, so we need a tokio runtime in scope. Enter the same
        // runtime used by the async surface; the spawned task lives
        // beyond this guard and runs on that runtime.
        let rt = pyo3_async_runtimes::tokio::get_runtime();
        let _g = rt.enter();
        Self {
            inner: Arc::new(ChannelHarness::in_memory()),
        }
    }

    /// Subscribe to lifecycle events.
    fn events(&self) -> PyChannelEventStream {
        PyChannelEventStream {
            inner: Arc::new(AsyncMutex::new(self.inner.events())),
        }
    }

    /// Attach an in-memory provider under `spec.channel_id`. The Python
    /// surface only supports the in-memory provider directly; production
    /// providers are wired from Rust where their concrete `Config` types
    /// can be parsed safely.
    fn attach_memory<'py>(
        &self,
        py: Python<'py>,
        provider: Py<PyInMemoryProvider>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let (provider_arc, channel_id) = {
            let p = provider.borrow(py);
            (p.inner.clone(), p.channel_id.clone())
        };
        let spec = ChannelSpec::new(channel_id.clone(), ProviderKind::Memory);
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner
                .attach_provider(spec, provider_arc)
                .await
                .map_err(rt_err)
        })
    }

    /// Detach a channel by id.
    fn detach<'py>(&self, py: Python<'py>, channel_id: String) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner
                .detach_provider(&ChannelId::from(channel_id))
                .await
                .map_err(rt_err)
        })
    }

    /// Open a thread bound to a `PyCallable`.
    fn open_thread<'py>(
        &self,
        py: Python<'py>,
        channel_id: String,
        peer: String,
        target: Py<PyCallable>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let handle: CallableHandle = target.borrow(py).inner.clone();
        let inner = self.inner.clone();
        let channel = ChannelId::from(channel_id);
        let peer = PeerId::from(peer);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let tref = inner
                .open_thread(&channel, peer, ThreadTarget::callable(handle))
                .await
                .map_err(rt_err)?;
            Python::with_gil(|py| Py::new(py, PyThreadRef { inner: tref }))
        })
    }

    fn close_thread<'py>(&self, py: Python<'py>, thread_id: String) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner
                .close_thread(&ThreadId::from(thread_id))
                .await
                .map_err(rt_err)
        })
    }

    /// Admin send — bypasses the bound target.
    fn send<'py>(
        &self,
        py: Python<'py>,
        thread_id: String,
        content: Py<PyMessageContent>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let mc = content.borrow(py).inner.clone();
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let ack = inner
                .send(&ThreadId::from(thread_id), mc)
                .await
                .map_err(rt_err)?;
            Python::with_gil(|py| {
                let d = PyDict::new_bound(py);
                d.set_item("provider_msg_id", ack.provider_msg_id)?;
                d.set_item("sent_at", ack.sent_at.to_rfc3339())?;
                Ok::<_, PyErr>(d.into_py(py))
            })
        })
    }

    fn list_channels<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let specs = inner.list_channels().await.map_err(rt_err)?;
            Python::with_gil(|py| {
                let arr = PyList::empty_bound(py);
                for s in specs {
                    let v = serde_json::to_value(&s).unwrap_or(Value::Null);
                    let d = json_to_py(py, &v)?;
                    arr.append(d)?;
                }
                Ok::<_, PyErr>(arr.into_py(py))
            })
        })
    }

    fn list_threads<'py>(&self, py: Python<'py>, channel_id: String) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let summaries = inner
                .list_threads(&ChannelId::from(channel_id))
                .await
                .map_err(rt_err)?;
            Python::with_gil(|py| {
                let arr = PyList::empty_bound(py);
                for s in summaries {
                    let v = serde_json::to_value(&s).unwrap_or(Value::Null);
                    let d = json_to_py(py, &v)?;
                    arr.append(d)?;
                }
                Ok::<_, PyErr>(arr.into_py(py))
            })
        })
    }

    fn list_messages<'py>(
        &self,
        py: Python<'py>,
        thread_id: String,
        limit: usize,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let recs = inner
                .list_messages(&ThreadId::from(thread_id), limit)
                .await
                .map_err(rt_err)?;
            Python::with_gil(|py| {
                let arr = PyList::empty_bound(py);
                for r in recs {
                    let v = serde_json::to_value(&r).unwrap_or(Value::Null);
                    let d = json_to_py(py, &v)?;
                    arr.append(d)?;
                }
                Ok::<_, PyErr>(arr.into_py(py))
            })
        })
    }

    /// Graceful shutdown — detaches every provider.
    fn shutdown<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner.shutdown().await.map_err(rt_err)
        })
    }

    fn __repr__(&self) -> String {
        format!(
            "ChannelHarness(attached={:?})",
            self.inner
                .list_attached()
                .into_iter()
                .map(|c| c.as_str().to_string())
                .collect::<Vec<_>>(),
        )
    }
}

// ----- registration -------------------------------------------------------

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "channel")?;
    m.add_class::<PyChannelHarness>()?;
    m.add_class::<PyChannelEventStream>()?;
    m.add_class::<PyInMemoryProvider>()?;
    m.add_class::<PyMessageContent>()?;
    m.add_class::<PyThreadRef>()?;
    parent.add_submodule(&m)?;
    Ok(())
}
