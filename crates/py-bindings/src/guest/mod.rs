//! Guest-mode plumbing — Python classes that implement Rust traits
//! (`Tool`, `InstructionStrategy`, `MemoryStrategy`, `SkillStrategy`,
//! `PersonaStrategy`, `MemoryStore`, `Embedder`, `Parser<T>`,
//! `Scorer<Outcome>`).
//!
//! The registration surface (`register_*_factory`) stores a
//! `Py<PyAny>` (the user's class or instance) in a process-wide
//! registry, returning an opaque `PyGuestHandle`. The corresponding
//! Rust adapter (one per submodule below) calls back into the GIL when
//! invoked, awaits the returned coroutine if any, and JSON-converts
//! the result.
//!
//! This intentionally does NOT depend on `atomr-pycore`'s
//! subinterpreter pool — that pool is the right answer for highly
//! parallel actor workloads, but agent turns are typically sequential
//! and the simpler in-process bridge avoids a transitive blast radius
//! of upstream actor-system deps. A subinterpreter-pool variant can
//! be added later as a feature-gated alternative dispatcher.

use std::sync::Arc;

use pyo3::prelude::*;

use crate::tool::PyToolDescriptor;

mod conv_helpers;
mod embedder;
mod instruction;
mod memory_store;
mod memory_strategy;
mod persona;
mod registry;
mod skill_strategy;
mod tool;

// NOTE: this crate enables `pyo3/extension-module`, so the `cargo
// test` binary cannot link to libpython — *any* `#[test]` block in
// this crate fails at link time even if it never touches a pyo3 type
// (the linker still pulls in pyo3-ffi as part of the rlib graph).
// Adapter behavior is covered by the Python pytest suite that ships
// alongside the wheel; the in-tree `cargo check -p
// atomr-agents-py-bindings` is the fast feedback loop here.

// Re-export the Python-facing handle types so other py-bindings
// modules (e.g. the eventual `Agent.from_spec` in W3b) can name them.
// The internal `PyXxxAdapter` structs are kept private to each
// submodule — callers consume them via the `Arc<dyn Trait>` held in
// the handle.
pub use embedder::PyEmbedder;
pub use instruction::PyInstruction;
pub use memory_store::PyMemoryStoreHandle;
pub use memory_strategy::PyMemoryStrategyHandle;
pub use persona::PyPersona;
pub use skill_strategy::PySkillStrategyHandle;

// Crate-internal re-exports of the strategy builders so sibling
// modules (notably `crate::agent::PyAgent::from_spec`) can construct
// strategies from registered guest keys without re-implementing the
// adapter wiring.
pub(crate) use instruction::build_guest_instruction_strategy;
pub(crate) use memory_strategy::build_guest_memory_strategy;
pub(crate) use persona::build_guest_persona;
pub(crate) use skill_strategy::build_guest_skill_strategy;
pub(crate) use tool::build_guest_toolset;

use registry::{ToolEntry, GUESTS, TOOLS};

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

pub(crate) fn register_kind(kind: &str, key: String, target: PyObject) -> PyGuestHandle {
    GUESTS.insert((kind.to_string(), key.clone()), Arc::new(target));
    PyGuestHandle {
        kind: kind.to_string(),
        key,
    }
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

/// Look up a registered Python target by `(kind, key)`. Returns `None`
/// if no entry exists. Used by builder functions in sibling modules
/// (e.g. [`crate::eval::build_guest_async_scorer`]) that turn a
/// registered Python class into a Rust trait-object adapter.
pub(crate) fn lookup_guest(kind: &str, key: &str) -> Option<std::sync::Arc<PyObject>> {
    GUESTS
        .get(&(kind.to_string(), key.to_string()))
        .map(|e| e.value().clone())
}

/// Sibling-module helper: look up a registered target or raise a
/// `KeyError`. Used by Phase-1+ submodules (callable, retriever,
/// ingest, embed, memory, persona, parser fixers, voice_extras,
/// observability tracers, etc.) when their `*_from_factory(key)`
/// builders materialise a Python target into a dyn handle.
pub(crate) fn must_lookup(kind: &str, key: &str) -> PyResult<std::sync::Arc<PyObject>> {
    lookup_guest(kind, key).ok_or_else(|| {
        pyo3::exceptions::PyKeyError::new_err(format!(
            "no guest factory registered under ({kind:?}, {key:?})"
        ))
    })
}

// ----- Extended factory registrations -------------------------------------
//
// 16 additional `register_X_factory` entry points consumed by the
// Phase-1+ submodules. Each pairs with a `@callable_` / `@retriever`
// / `@loader` / … decorator in `python/atomr_agents/guest.py`.

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

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "guest")?;
    m.add_class::<PyGuestHandle>()?;
    m.add_class::<PyInstruction>()?;
    m.add_class::<PyMemoryStrategyHandle>()?;
    m.add_class::<PySkillStrategyHandle>()?;
    m.add_class::<PyPersona>()?;
    m.add_class::<PyMemoryStoreHandle>()?;
    m.add_class::<PyEmbedder>()?;

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

    m.add_function(wrap_pyfunction!(tool::build_guest_toolset, &m)?)?;
    m.add_function(wrap_pyfunction!(
        instruction::build_guest_instruction_strategy,
        &m
    )?)?;
    m.add_function(wrap_pyfunction!(
        memory_strategy::build_guest_memory_strategy,
        &m
    )?)?;
    m.add_function(wrap_pyfunction!(
        skill_strategy::build_guest_skill_strategy,
        &m
    )?)?;
    m.add_function(wrap_pyfunction!(persona::build_guest_persona, &m)?)?;
    m.add_function(wrap_pyfunction!(memory_store::build_guest_memory_store, &m)?)?;
    m.add_function(wrap_pyfunction!(embedder::build_guest_embedder, &m)?)?;

    parent.add_submodule(&m)?;
    Ok(())
}
