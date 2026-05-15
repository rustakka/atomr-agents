//! Python class → Rust `Tool` adapter and the `ToolSet` builder.
//!
//! Wraps a Python class/instance as a Rust `Tool` impl. On `invoke`
//! we acquire the GIL, instantiate the class if needed, call its
//! `invoke(args, ctx)` method, and (if the return is a coroutine)
//! await it via `pyo3-async-runtimes::tokio::into_future`.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{InvokeCtx, Result as AgentResult, Value};
use atomr_agents_tool::{DynTool, Tool, ToolDescriptor};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::conv::json_to_py;
use crate::tool::PyToolSet;

use super::registry::TOOLS;

pub struct PyToolAdapter {
    descriptor: ToolDescriptor,
    target: Arc<PyObject>,
}

impl PyToolAdapter {
    pub(crate) fn new(descriptor: ToolDescriptor, target: Arc<PyObject>) -> Self {
        Self { descriptor, target }
    }

    fn build_ctx_dict<'py>(py: Python<'py>, ctx: &InvokeCtx) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new_bound(py);
        d.set_item("tool_call_id", &ctx.tool_call_id)?;
        if let Some(aid) = &ctx.call.agent_id {
            d.set_item("agent_id", aid.as_str())?;
        }
        d.set_item("trace", ctx.call.trace.clone())?;
        d.set_item("tokens_remaining", ctx.call.tokens.remaining)?;
        d.set_item("time_ms_remaining", ctx.call.time.remaining_ms)?;
        d.set_item("iterations_remaining", ctx.call.iterations.remaining)?;
        Ok(d)
    }
}

#[async_trait]
impl Tool for PyToolAdapter {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn invoke(&self, args: Value, ctx: &InvokeCtx) -> AgentResult<Value> {
        // Acquire GIL, invoke the Python callable, optionally await
        // the returned coroutine on the tokio runtime.
        let target = self.target.clone();
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let args_obj = json_to_py(py, &args)?;
            let ctx_dict = Self::build_ctx_dict(py, ctx)?;
            let bound = target.bind(py);
            // If the target is a class, instantiate it (zero-arg ctor).
            // Otherwise treat it as an already-callable object.
            let instance: Bound<'_, PyAny> = if bound.is_callable() && bound.hasattr("__call__")? {
                // Heuristic: a *class* has `__init__` AND `__name__`
                // and is itself callable; an *instance* exposes
                // `invoke`. Prefer calling `invoke` on an instance.
                if bound.hasattr("invoke")? {
                    bound.clone()
                } else {
                    bound.call0()?
                }
            } else {
                bound.clone()
            };
            let invoke_attr = instance.getattr("invoke")?;
            let result = invoke_attr.call1((args_obj, ctx_dict))?;
            Ok(result.unbind())
        })
        .map_err(|e| atomr_agents_core::AgentError::Tool(format!("guest tool: {e}")))?;

        // If the value is a coroutine, await it. We branch here
        // because `into_future` requires `Bound<PyAny>` referencing
        // the coroutine and converts it to a Send future.
        let final_val = {
            let maybe_future = Python::with_gil(|py| -> PyResult<Option<_>> {
                let bound = coro_or_val.bind(py);
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
            .map_err(|e| atomr_agents_core::AgentError::Tool(format!("guest tool coroutine: {e}")))?;

            match maybe_future {
                Some(fut) => fut
                    .await
                    .map_err(|e| atomr_agents_core::AgentError::Tool(format!("guest tool await: {e}")))?,
                None => coro_or_val,
            }
        };

        // Convert the Python result back to JSON.
        let v = Python::with_gil(|py| crate::conv::py_to_json(py, final_val.bind(py)))
            .map_err(|e| atomr_agents_core::AgentError::Tool(format!("guest tool result: {e}")))?;
        Ok(v)
    }
}

// ----- ToolSet builder for guest tools --------------------------------------
//
// Looks up registered tool factories by key, builds adapters, and
// produces a Rust-side `ToolSet` that callers can plug into agent
// runners. Exposes both "from a list of keys" and "register all
// known" entrypoints.

#[pyfunction]
#[pyo3(signature = (id, version, keys=None))]
pub(crate) fn build_guest_toolset(
    id: String,
    version: &str,
    keys: Option<Vec<String>>,
) -> PyResult<PyToolSet> {
    use atomr_agents_core::ToolSetId;
    use atomr_agents_tool::ToolSet;
    use semver::Version;

    let v: Version = crate::conv::parse_version(version)?;
    let selected = keys.unwrap_or_else(|| TOOLS.iter().map(|e| e.key().clone()).collect());

    let mut tools: Vec<DynTool> = Vec::with_capacity(selected.len());
    for key in &selected {
        let entry = TOOLS.get(key).ok_or_else(|| {
            pyo3::exceptions::PyKeyError::new_err(format!("no guest tool registered with key {key:?}"))
        })?;
        let adapter = PyToolAdapter::new(entry.descriptor.clone(), entry.target.clone());
        tools.push(Arc::new(adapter));
    }

    Ok(PyToolSet {
        inner: Arc::new(ToolSet::new(ToolSetId::from(id), v, tools)),
    })
}
