//! Python-backed [`AgentSdkBackend`] + the reverse async-iterator bridge.
//!
//! `PythonAgentSdkBackend` implements the harness crate's backend trait by
//! delegating to a registered Python `ClaudeAgentSDKBackend` wrapper (see
//! `python/atomr_agents/agent_sdk.py`). The crux is [`aiter_to_stream`]:
//! Rust drives a Python async iterator (`__anext__`) one item at a time,
//! awaiting each coroutine via `pyo3_async_runtimes::tokio::into_future`
//! — the inverse of `observability::PyEventStream`.
//!
//! GIL discipline (each independently load-bearing): the GIL is held only
//! to *build* the coroutine / *normalize* the result; the `.await` always
//! happens with the GIL released. `StopAsyncIteration` is the terminator.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use pyo3::exceptions::PyStopAsyncIteration;
use pyo3::prelude::*;

use atomr_agents_agent_sdk_core::{
    AgentSdkBackend, AgentSdkError, AgentSdkMessage, AgentSdkSession, AgentSessionId, MessageStream,
    QueryRequest, SessionSpec,
};
use atomr_agents_core::{
    CallCtx, InvokeCtx, IterationBudget, MoneyBudget, TimeBudget, TokenBudget,
};
use atomr_agents_tool::Tool;

use crate::conv::{json_to_py, py_to_json};

/// Convert a registered Python async iterator into a Rust [`MessageStream`].
fn aiter_to_stream(aiter: Arc<PyObject>) -> MessageStream {
    futures::stream::unfold(Some(aiter), |state: Option<Arc<PyObject>>| async move {
        let aiter = state?;
        // (A) GIL: __anext__() → coroutine → Send future.
        let step = Python::with_gil(|py| -> PyResult<_> {
            let coro = aiter.bind(py).call_method0("__anext__")?;
            pyo3_async_runtimes::tokio::into_future(coro)
        });
        let fut = match step {
            Ok(f) => f,
            Err(e) => return Some((Err(AgentSdkError::Sdk(format!("__anext__: {e}"))), None)),
        };
        // (B) await OUTSIDE the GIL.
        match fut.await {
            Ok(obj) => {
                // (C) GIL: normalize → JSON → AgentSdkMessage.
                let parsed = Python::with_gil(|py| py_to_json(py, obj.bind(py)));
                match parsed {
                    Ok(value) => {
                        let msg = serde_json::from_value::<AgentSdkMessage>(value)
                            .unwrap_or(AgentSdkMessage::Unknown);
                        Some((Ok(msg), Some(aiter)))
                    }
                    Err(e) => Some((Err(AgentSdkError::Sdk(format!("normalize: {e}"))), None)),
                }
            }
            Err(e) => {
                let is_stop = Python::with_gil(|py| e.is_instance_of::<PyStopAsyncIteration>(py));
                if is_stop {
                    None
                } else {
                    Some((Err(AgentSdkError::Sdk(format!("stream: {e}"))), None))
                }
            }
        }
    })
    .boxed()
}

/// Await a Python coroutine returned by a method on `target`. The method is
/// called with `(token)` or `(token, arg)`.
async fn call_void(
    target: &Arc<PyObject>,
    method: &'static str,
    token: &str,
    arg: Option<String>,
) -> Result<(), AgentSdkError> {
    let target = target.clone();
    let token = token.to_string();
    let fut = Python::with_gil(|py| -> PyResult<_> {
        let bound = target.bind(py);
        let coro = match arg {
            Some(a) => bound.call_method1(method, (token, a))?,
            None => bound.call_method1(method, (token,))?,
        };
        pyo3_async_runtimes::tokio::into_future(coro)
    })
    .map_err(|e| AgentSdkError::Sdk(format!("{method}: {e}")))?;
    fut.await
        .map_err(|e| AgentSdkError::Sdk(format!("{method} await: {e}")))?;
    Ok(())
}

/// The backend: holds the registered Python wrapper instance.
pub(crate) struct PythonAgentSdkBackend {
    target: Arc<PyObject>,
}

#[async_trait]
impl AgentSdkBackend for PythonAgentSdkBackend {
    fn name(&self) -> &str {
        "python"
    }

    async fn available(&self) -> bool {
        true
    }

    async fn query(&self, req: QueryRequest) -> Result<MessageStream, AgentSdkError> {
        let cfg = serde_json::to_value(&req.config)?;
        let prompt = req.prompt;
        let target = self.target.clone();
        let aiter = Python::with_gil(|py| -> PyResult<Arc<PyObject>> {
            let cfg_py = json_to_py(py, &cfg)?;
            let agen = target.bind(py).call_method1("run", (cfg_py, prompt))?;
            let aiter = agen.call_method0("__aiter__")?;
            Ok(Arc::new(aiter.unbind()))
        })
        .map_err(|e| AgentSdkError::Sdk(format!("run: {e}")))?;
        Ok(aiter_to_stream(aiter))
    }

    async fn create_session(
        &self,
        spec: SessionSpec,
    ) -> Result<Box<dyn AgentSdkSession>, AgentSdkError> {
        let cfg = serde_json::to_value(&spec.config)?;
        let target = self.target.clone();
        let fut = Python::with_gil(|py| -> PyResult<_> {
            let cfg_py = json_to_py(py, &cfg)?;
            let coro = target.bind(py).call_method1("open_session", (cfg_py,))?;
            pyo3_async_runtimes::tokio::into_future(coro)
        })
        .map_err(|e| AgentSdkError::Sdk(format!("open_session: {e}")))?;
        let token_obj = fut
            .await
            .map_err(|e| AgentSdkError::Sdk(format!("open_session await: {e}")))?;
        let token: String = Python::with_gil(|py| token_obj.bind(py).extract())
            .map_err(|e| AgentSdkError::Sdk(format!("session token: {e}")))?;
        let id = AgentSessionId::from(token.clone());
        Ok(Box::new(PythonAgentSession {
            target: self.target.clone(),
            token,
            id,
        }))
    }
}

/// A live Python-backed session. The stateful `ClaudeSDKClient` stays
/// Python-side, keyed by an opaque `token`; Rust holds only the token.
pub(crate) struct PythonAgentSession {
    target: Arc<PyObject>,
    token: String,
    id: AgentSessionId,
}

#[async_trait]
impl AgentSdkSession for PythonAgentSession {
    fn session_id(&self) -> &AgentSessionId {
        &self.id
    }

    async fn send(&self, prompt: String) -> Result<(), AgentSdkError> {
        call_void(&self.target, "session_send", &self.token, Some(prompt)).await
    }

    async fn receive(&self) -> Result<MessageStream, AgentSdkError> {
        let target = self.target.clone();
        let token = self.token.clone();
        let aiter = Python::with_gil(|py| -> PyResult<Arc<PyObject>> {
            let agen = target.bind(py).call_method1("session_stream", (token,))?;
            let aiter = agen.call_method0("__aiter__")?;
            Ok(Arc::new(aiter.unbind()))
        })
        .map_err(|e| AgentSdkError::Sdk(format!("session_stream: {e}")))?;
        Ok(aiter_to_stream(aiter))
    }

    async fn interrupt(&self) -> Result<(), AgentSdkError> {
        call_void(&self.target, "session_interrupt", &self.token, None).await
    }

    async fn set_permission_mode(&self, mode: String) -> Result<(), AgentSdkError> {
        call_void(&self.target, "session_set_mode", &self.token, Some(mode)).await
    }

    async fn set_model(&self, model: String) -> Result<(), AgentSdkError> {
        call_void(&self.target, "session_set_model", &self.token, Some(model)).await
    }

    async fn close(&self) -> Result<(), AgentSdkError> {
        call_void(&self.target, "session_close", &self.token, None).await
    }
}

/// Build a Python-backed backend from a registered `agent_sdk` factory key.
pub(crate) fn build_agent_sdk_backend(key: &str) -> PyResult<Arc<dyn AgentSdkBackend>> {
    let target = super::must_lookup("agent_sdk", key)?;
    Ok(Arc::new(PythonAgentSdkBackend { target }))
}

/// Invoke a registered atomr tool (a `@tool` guest) by key and return its
/// result — the bridge that lets a Rust/Python atomr tool run as an
/// in-process SDK custom tool. Async; runs through the Rust `Tool` trait.
#[pyfunction]
#[pyo3(signature = (key, args, ctx=None))]
pub(crate) fn invoke_tool<'py>(
    py: Python<'py>,
    key: String,
    args: &Bound<'py, PyAny>,
    ctx: Option<&Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyAny>> {
    let args_val = py_to_json(py, args)?;
    let _ = ctx; // reserved for future per-call budget/clearance plumbing
    let adapter = {
        let entry = super::registry::TOOLS.get(&key).ok_or_else(|| {
            pyo3::exceptions::PyKeyError::new_err(format!("no atomr tool registered with key {key:?}"))
        })?;
        super::tool::PyToolAdapter::new(entry.descriptor.clone(), entry.target.clone())
    };
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        let ictx = InvokeCtx {
            call: CallCtx::new(
                None,
                TokenBudget::new(1_000_000),
                TimeBudget::new(Duration::from_secs(3600)),
                MoneyBudget::from_usd(1_000.0),
                IterationBudget::new(1_000),
                vec![],
            ),
            tool_call_id: "agent-sdk".to_string(),
            raw_args: args_val.clone(),
        };
        let result = adapter
            .invoke(args_val, &ictx)
            .await
            .map_err(crate::errors::map)?;
        Python::with_gil(|py| json_to_py(py, &result))
    })
}
