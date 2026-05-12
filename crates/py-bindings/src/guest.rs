//! Guest-mode plumbing — Python classes that implement Rust traits
//! (`Tool`, `ContextStrategy`, `PersonaStrategy`, `SkillStrategy`,
//! `Parser<T>`, `Scorer<Outcome>`, `MemoryStore`, `Embedder`).
//!
//! Today we ship the *registration surface* — `register_tool_factory`
//! et al — that the Python `guest.py` decorators call when the
//! `@tool` / `@strategy` / `@persona` markers are applied. Each
//! factory stores a `Py<PyAny>` (the user's class or instance) in a
//! process-wide registry; the corresponding Rust adapter
//! (`PyToolAdapter` for tools, more on the way) calls back into the
//! GIL when invoked.
//!
//! This intentionally does NOT depend on `atomr-pycore`'s
//! subinterpreter pool — that pool is the right answer for highly
//! parallel actor workloads, but agent turns are typically sequential
//! and the simpler in-process bridge avoids a transitive blast radius
//! of upstream actor-system deps. A subinterpreter-pool variant can
//! be added later as a feature-gated alternative dispatcher.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{InvokeCtx, Result as AgentResult, Value};
use atomr_agents_tool::{DynTool, Tool, ToolDescriptor};
use dashmap::DashMap;
use once_cell::sync::Lazy;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::conv::{json_to_py, py_to_json};
use crate::tool::{PyToolDescriptor, PyToolSet};

/// Shared handle returned to Python after registration. Holds the
/// user's class/instance + a stable string key.
#[pyclass(name = "GuestHandle", module = "atomr_agents._native.guest")]
#[derive(Clone)]
pub struct PyGuestHandle {
    #[pyo3(get)]
    pub kind: String,
    #[pyo3(get)]
    pub key: String,
}

#[pymethods]
impl PyGuestHandle {
    fn __repr__(&self) -> String {
        format!("GuestHandle(kind={:?}, key={:?})", self.kind, self.key)
    }
}

/// Per-tool registry entry: descriptor + the Python target.
#[derive(Clone)]
struct ToolEntry {
    descriptor: ToolDescriptor,
    target: Arc<PyObject>,
}

/// Process-wide registries. Generic factories store any PyObject;
/// tool factories additionally carry their descriptor so the Rust
/// adapter advertises the right schema.
static GUESTS: Lazy<DashMap<(String, String), Arc<PyObject>>> = Lazy::new(DashMap::new);
static TOOLS: Lazy<DashMap<String, ToolEntry>> = Lazy::new(DashMap::new);

fn register_kind(kind: &str, key: String, target: PyObject) -> PyGuestHandle {
    GUESTS.insert((kind.to_string(), key.clone()), Arc::new(target));
    PyGuestHandle {
        kind: kind.to_string(),
        key,
    }
}

/// Look up a registered guest target by `(kind, key)`. Returns the
/// shared `Arc<PyObject>` if registered. Used by per-domain submodules
/// to materialise adapters on demand.
pub fn lookup(kind: &str, key: &str) -> Option<Arc<PyObject>> {
    GUESTS
        .get(&(kind.to_string(), key.to_string()))
        .map(|e| e.value().clone())
}

/// Look up a registered guest target or return an error.
pub fn must_lookup(kind: &str, key: &str) -> PyResult<Arc<PyObject>> {
    lookup(kind, key).ok_or_else(|| {
        pyo3::exceptions::PyKeyError::new_err(format!(
            "no guest factory registered under ({kind:?}, {key:?})"
        ))
    })
}

#[pyfunction]
#[pyo3(signature = (key, target, descriptor=None))]
fn register_tool_factory(
    key: String,
    target: PyObject,
    descriptor: Option<PyToolDescriptor>,
) -> PyGuestHandle {
    let target = Arc::new(target);
    if let Some(d) = descriptor {
        TOOLS.insert(
            key.clone(),
            ToolEntry {
                descriptor: d.inner,
                target: target.clone(),
            },
        );
    }
    GUESTS.insert(("tool".to_string(), key.clone()), target);
    PyGuestHandle {
        kind: "tool".to_string(),
        key,
    }
}

#[pyfunction]
fn register_strategy_factory(kind: String, key: String, target: PyObject) -> PyGuestHandle {
    register_kind(&format!("strategy:{kind}"), key, target)
}

#[pyfunction]
fn register_persona_factory(key: String, target: PyObject) -> PyGuestHandle {
    register_kind("persona", key, target)
}

#[pyfunction]
fn register_skill_factory(key: String, target: PyObject) -> PyGuestHandle {
    register_kind("skill", key, target)
}

#[pyfunction]
fn register_parser_factory(key: String, target: PyObject) -> PyGuestHandle {
    register_kind("parser", key, target)
}

#[pyfunction]
fn register_scorer_factory(key: String, target: PyObject) -> PyGuestHandle {
    register_kind("scorer", key, target)
}

#[pyfunction]
fn register_memory_factory(key: String, target: PyObject) -> PyGuestHandle {
    register_kind("memory", key, target)
}

#[pyfunction]
fn register_embedder_factory(key: String, target: PyObject) -> PyGuestHandle {
    register_kind("embedder", key, target)
}

// ----- New factory registrations (Phase 0 stubs) ----------------------------

#[pyfunction]
fn register_callable_factory(key: String, target: PyObject) -> PyGuestHandle {
    register_kind("callable", key, target)
}

#[pyfunction]
fn register_retriever_factory(key: String, target: PyObject) -> PyGuestHandle {
    register_kind("retriever", key, target)
}

#[pyfunction]
fn register_loader_factory(key: String, target: PyObject) -> PyGuestHandle {
    register_kind("loader", key, target)
}

#[pyfunction]
fn register_splitter_factory(key: String, target: PyObject) -> PyGuestHandle {
    register_kind("splitter", key, target)
}

#[pyfunction]
fn register_kv_cache_factory(key: String, target: PyObject) -> PyGuestHandle {
    register_kind("kv_cache", key, target)
}

#[pyfunction]
fn register_long_store_factory(key: String, target: PyObject) -> PyGuestHandle {
    register_kind("long_store", key, target)
}

#[pyfunction]
fn register_tracer_factory(key: String, target: PyObject) -> PyGuestHandle {
    register_kind("tracer", key, target)
}

#[pyfunction]
fn register_conversation_agent_factory(key: String, target: PyObject) -> PyGuestHandle {
    register_kind("conversation_agent", key, target)
}

#[pyfunction]
fn register_diarizer_factory(key: String, target: PyObject) -> PyGuestHandle {
    register_kind("diarizer", key, target)
}

#[pyfunction]
fn register_vad_factory(key: String, target: PyObject) -> PyGuestHandle {
    register_kind("vad", key, target)
}

#[pyfunction]
fn register_phonemizer_factory(key: String, target: PyObject) -> PyGuestHandle {
    register_kind("phonemizer", key, target)
}

#[pyfunction]
fn register_journal_factory(key: String, target: PyObject) -> PyGuestHandle {
    register_kind("journal", key, target)
}

#[pyfunction]
fn register_repair_model_factory(key: String, target: PyObject) -> PyGuestHandle {
    register_kind("repair_model", key, target)
}

#[pyfunction]
fn register_persona_reconciler_factory(key: String, target: PyObject) -> PyGuestHandle {
    register_kind("persona_reconciler", key, target)
}

#[pyfunction]
fn register_inference_client_factory(key: String, target: PyObject) -> PyGuestHandle {
    register_kind("inference_client", key, target)
}

#[pyfunction]
fn register_ann_index_factory(key: String, target: PyObject) -> PyGuestHandle {
    register_kind("ann_index", key, target)
}

#[pyfunction]
fn list_factories(kind: String) -> Vec<String> {
    // Tool factories may be registered with or without a descriptor.
    // When a descriptor is supplied they live in `TOOLS`; otherwise
    // they live in `GUESTS` under the "tool" kind. Merge both so
    // `list_factories("tool")` reports every registered tool.
    let mut out: Vec<String> = GUESTS
        .iter()
        .filter(|e| e.key().0 == kind)
        .map(|e| e.key().1.clone())
        .collect();
    if kind == "tool" {
        for e in TOOLS.iter() {
            let k = e.key().clone();
            if !out.contains(&k) {
                out.push(k);
            }
        }
    }
    out
}

#[pyfunction]
fn clear_factories() -> usize {
    let n = GUESTS.len() + TOOLS.len();
    GUESTS.clear();
    TOOLS.clear();
    n
}

// ----- Tool adapter ---------------------------------------------------------
//
// Wraps a Python class/instance as a Rust `Tool` impl. On `invoke`,
// we acquire the GIL, instantiate the class if needed, call its
// `invoke(args, ctx)` method, and (if the return is a coroutine)
// await it via `pyo3-async-runtimes::tokio::into_future`.

pub struct PyToolAdapter {
    descriptor: ToolDescriptor,
    target: Arc<PyObject>,
}

impl PyToolAdapter {
    fn new(descriptor: ToolDescriptor, target: Arc<PyObject>) -> Self {
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

        // If it's a coroutine, await it; otherwise treat the return
        // value as the result directly.
        let returned = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = coro_or_val.bind(py);
            let inspect = py.import_bound("inspect")?;
            let iscoroutine = inspect.getattr("iscoroutine")?;
            let is_coro: bool = iscoroutine.call1((bound,))?.extract()?;
            if is_coro {
                // Defer to the async path below — return the
                // coroutine wrapped as a future.
                Ok(coro_or_val.clone_ref(py))
            } else {
                Ok(coro_or_val.clone_ref(py))
            }
        })
        .map_err(|e| atomr_agents_core::AgentError::Tool(format!("guest tool: {e}")))?;

        // If the value is a coroutine, await it. We branch here
        // because `into_future` requires `Bound<PyAny>` referencing
        // the coroutine and converts it to a Send future.
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
                atomr_agents_core::AgentError::Tool(format!("guest tool coroutine: {e}"))
            })?;

            match maybe_future {
                Some(fut) => fut.await.map_err(|e| {
                    atomr_agents_core::AgentError::Tool(format!("guest tool await: {e}"))
                })?,
                None => returned,
            }
        };

        // Convert the Python result back to JSON.
        let v = Python::with_gil(|py| py_to_json(py, final_val.bind(py)))
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
fn build_guest_toolset(
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
            pyo3::exceptions::PyKeyError::new_err(format!(
                "no guest tool registered with key {key:?}"
            ))
        })?;
        let adapter = PyToolAdapter::new(entry.descriptor.clone(), entry.target.clone());
        tools.push(Arc::new(adapter));
    }

    Ok(PyToolSet {
        inner: Arc::new(ToolSet::new(ToolSetId::from(id), v, tools)),
    })
}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "guest")?;
    m.add_class::<PyGuestHandle>()?;
    m.add_function(wrap_pyfunction!(register_tool_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(register_strategy_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(register_persona_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(register_skill_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(register_parser_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(register_scorer_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(register_memory_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(register_embedder_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(register_callable_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(register_retriever_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(register_loader_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(register_splitter_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(register_kv_cache_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(register_long_store_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(register_tracer_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(register_conversation_agent_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(register_diarizer_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(register_vad_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(register_phonemizer_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(register_journal_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(register_repair_model_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(register_persona_reconciler_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(register_inference_client_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(register_ann_index_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(list_factories, &m)?)?;
    m.add_function(wrap_pyfunction!(clear_factories, &m)?)?;
    m.add_function(wrap_pyfunction!(build_guest_toolset, &m)?)?;
    parent.add_submodule(&m)?;
    Ok(())
}
