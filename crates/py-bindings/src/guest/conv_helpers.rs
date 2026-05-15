//! Shared helpers for the Python→Rust guest adapters.
//!
//! Each adapter (instruction, persona, memory strategy / store, skill,
//! embedder) needs the same plumbing: project Rust types
//! (`AgentContext`, `TokenBudget`, `MemoryItem`) into Python dicts so
//! the user's class can read them, detect whether the call returned a
//! coroutine, and await it on the shared tokio runtime. Centralizing
//! that here keeps each adapter file under 100 LOC.

use atomr_agents_core::{AgentContext, MemoryItem, MemoryKind, MemoryNamespace, TokenBudget, Value};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::conv::{json_to_py, py_to_json};

/// Project an `AgentContext` to a Python dict. Mirrors
/// `PyToolAdapter::build_ctx_dict` (originally in guest.rs:178-189) but
/// for the per-turn pipeline context, not the per-tool invoke context.
pub(crate) fn build_agent_ctx_dict<'py>(py: Python<'py>, ctx: &AgentContext) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new_bound(py);
    d.set_item("agent_id", ctx.agent_id.as_str())?;
    if let Some(t) = &ctx.team_id {
        d.set_item("team_id", t.as_str())?;
    }
    if let Some(o) = &ctx.org_id {
        d.set_item("org_id", o.as_str())?;
    }
    let turn = PyDict::new_bound(py);
    turn.set_item("user", &ctx.turn.user)?;
    let history = pyo3::types::PyList::empty_bound(py);
    for m in &ctx.turn.history {
        let msg = PyDict::new_bound(py);
        let role = match m.role {
            atomr_agents_core::MessageRole::System => "system",
            atomr_agents_core::MessageRole::User => "user",
            atomr_agents_core::MessageRole::Assistant => "assistant",
            atomr_agents_core::MessageRole::Tool => "tool",
        };
        msg.set_item("role", role)?;
        msg.set_item("content", &m.content)?;
        history.append(msg)?;
    }
    turn.set_item("history", history)?;
    d.set_item("turn", turn)?;
    Ok(d)
}

/// Project a `TokenBudget` to a Python dict. Strategies normally only
/// read `remaining`, but we include both fields for parity with the
/// Rust struct.
pub(crate) fn build_budget_dict<'py>(py: Python<'py>, budget: &TokenBudget) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new_bound(py);
    d.set_item("remaining", budget.remaining)?;
    d.set_item("reserved", budget.reserved)?;
    Ok(d)
}

/// Project a `MemoryItem` to a Python dict. Used by the
/// `MemoryStore::list` adapter return path.
pub(crate) fn build_memory_item_dict<'py>(
    py: Python<'py>,
    item: &MemoryItem,
) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new_bound(py);
    d.set_item("id", &item.id)?;
    d.set_item("kind", memory_kind_to_str(item.kind))?;
    d.set_item("namespace", build_namespace_dict(py, &item.namespace)?)?;
    d.set_item("payload", json_to_py(py, &item.payload)?)?;
    d.set_item("timestamp_ms", item.timestamp_ms)?;
    let tags = pyo3::types::PyList::empty_bound(py);
    for t in &item.tags {
        tags.append(t)?;
    }
    d.set_item("tags", tags)?;
    Ok(d)
}

fn memory_kind_to_str(k: MemoryKind) -> &'static str {
    match k {
        MemoryKind::Episodic => "episodic",
        MemoryKind::Semantic => "semantic",
        MemoryKind::Working => "working",
        MemoryKind::Scratchpad => "scratchpad",
    }
}

fn memory_kind_from_str(s: &str) -> Option<MemoryKind> {
    match s {
        "episodic" => Some(MemoryKind::Episodic),
        "semantic" => Some(MemoryKind::Semantic),
        "working" => Some(MemoryKind::Working),
        "scratchpad" => Some(MemoryKind::Scratchpad),
        _ => None,
    }
}

fn build_namespace_dict<'py>(py: Python<'py>, ns: &MemoryNamespace) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new_bound(py);
    match ns {
        MemoryNamespace::Agent(a) => {
            d.set_item("scope", "agent")?;
            d.set_item("id", a.as_str())?;
        }
        MemoryNamespace::Team(t) => {
            d.set_item("scope", "team")?;
            d.set_item("id", t.as_str())?;
        }
        MemoryNamespace::Org(o) => {
            d.set_item("scope", "org")?;
            d.set_item("id", o.as_str())?;
        }
    }
    Ok(d)
}

/// Parse a Python object (typically a dict) back into a `MemoryItem`.
/// Used by the `MemoryStore::list` adapter return path. Reuses
/// `py_to_json` for the heavy lifting then validates field shapes.
#[allow(dead_code)] // used by future adapters expecting Python input
pub(crate) fn parse_memory_item(_py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<MemoryItem> {
    let v = py_to_json(obj.py(), obj)?;
    parse_memory_item_value(&v)
}

pub(crate) fn parse_memory_item_value(v: &Value) -> PyResult<MemoryItem> {
    let map = v
        .as_object()
        .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("memory item must be an object"))?;
    let id = map
        .get("id")
        .and_then(|x| x.as_str())
        .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("memory item missing id"))?
        .to_string();
    let kind_str = map
        .get("kind")
        .and_then(|x| x.as_str())
        .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("memory item missing kind"))?;
    let kind = memory_kind_from_str(kind_str).ok_or_else(|| {
        pyo3::exceptions::PyValueError::new_err(format!("unknown memory kind {kind_str:?}"))
    })?;
    let ns = parse_namespace_value(
        map.get("namespace")
            .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("memory item missing namespace"))?,
    )?;
    let payload = map.get("payload").cloned().unwrap_or(serde_json::Value::Null);
    let timestamp_ms = map.get("timestamp_ms").and_then(|x| x.as_i64()).unwrap_or(0);
    let tags: Vec<String> = map
        .get("tags")
        .and_then(|x| x.as_array())
        .map(|arr| arr.iter().filter_map(|t| t.as_str().map(String::from)).collect())
        .unwrap_or_default();
    Ok(MemoryItem {
        id,
        kind,
        namespace: ns,
        payload,
        timestamp_ms,
        tags,
    })
}

fn parse_namespace_value(v: &Value) -> PyResult<MemoryNamespace> {
    let map = v
        .as_object()
        .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("memory namespace must be an object"))?;
    let scope = map
        .get("scope")
        .and_then(|x| x.as_str())
        .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("namespace missing scope"))?;
    let id = map
        .get("id")
        .and_then(|x| x.as_str())
        .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("namespace missing id"))?;
    Ok(match scope {
        "agent" => MemoryNamespace::Agent(atomr_agents_core::AgentId::from(id)),
        "team" => MemoryNamespace::Team(atomr_agents_core::TeamId::from(id)),
        "org" => MemoryNamespace::Org(atomr_agents_core::OrgId::from(id)),
        other => {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "unknown namespace scope {other:?}"
            )));
        }
    })
}

/// Project a `MemoryNamespace` to the same dict shape that
/// `parse_namespace_value` reads. Used by `MemoryStore::list` to
/// communicate the namespace argument to Python.
pub(crate) fn build_namespace_dict_pub<'py>(
    py: Python<'py>,
    ns: &MemoryNamespace,
) -> PyResult<Bound<'py, PyDict>> {
    build_namespace_dict(py, ns)
}

/// Call `inspect.iscoroutine(obj)` on the GIL thread. Returns whether
/// `obj` is a Python coroutine that needs awaiting via `into_future`.
pub(crate) fn is_coroutine(py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<bool> {
    let inspect = py.import_bound("inspect")?;
    let iscoroutine = inspect.getattr("iscoroutine")?;
    iscoroutine.call1((obj,))?.extract()
}

/// Resolve a registered Python target into the bound *callable*.
/// Mirrors the heuristic in `PyToolAdapter::invoke`: if the target has
/// the named method directly (i.e. it's an instance), use it as-is;
/// otherwise treat it as a class and invoke the zero-arg ctor first.
pub(crate) fn resolve_instance<'py>(target: &Bound<'py, PyAny>, method: &str) -> PyResult<Bound<'py, PyAny>> {
    if target.hasattr(method)? {
        Ok(target.clone())
    } else if target.is_callable() {
        target.call0()
    } else {
        Err(pyo3::exceptions::PyTypeError::new_err(format!(
            "guest target is not callable and has no `{method}` method",
        )))
    }
}

/// Acquire a coroutine return from a Python call, then await it on the
/// tokio runtime if it is one. The result is returned as a JSON
/// `Value` (the canonical Rust↔Python boundary).
pub(crate) async fn await_and_jsonify(returned: PyObject) -> PyResult<Value> {
    // Detect coroutine inside the GIL; if so, hand off to into_future.
    let maybe_future = Python::with_gil(|py| -> PyResult<Option<_>> {
        let bound = returned.bind(py);
        if is_coroutine(py, bound)? {
            Ok(Some(pyo3_async_runtimes::tokio::into_future(bound.clone())?))
        } else {
            Ok(None)
        }
    })?;

    let final_obj = match maybe_future {
        Some(fut) => fut
            .await
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?,
        None => returned,
    };

    Python::with_gil(|py| py_to_json(py, final_obj.bind(py)))
}
