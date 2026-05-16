//! # atomr-agents-py-bindings
//!
//! PyO3 bindings exposing atomr-agents' data types, agent runtime,
//! workflow / harness orchestrators, registry, observability, and
//! guest-mode plumbing for Python-implementable Rust traits.
//!
//! Module layout (mirrors upstream `atomr-infer/inference-py-bindings`
//! and `atomr/pycore`):
//!
//! - `atomr_agents._native.errors`        — exception hierarchy
//! - `atomr_agents._native.core`          — IDs, budgets, message /
//!                                          memory primitives,
//!                                          inference re-exports
//! - `atomr_agents._native.observability` — `EventBus`, `Event`,
//!                                          async `EventStream`,
//!                                          `RunTreeBuilder`
//! - `atomr_agents._native.registry`      — `Registry`,
//!                                          `ArtifactKind`,
//!                                          `ArtifactRecord`,
//!                                          `EvalSummary`
//! - `atomr_agents._native.tool`          — `ToolDescriptor`,
//!                                          `ToolSet`,
//!                                          `ToolCallParser`
//! - `atomr_agents._native.skill`         — `Skill`, `SkillSet`
//! - `atomr_agents._native.persona`       — Persona variants
//! - `atomr_agents._native.agent`         — `AgentSpec`,
//!                                          `AgentBudgets`
//! - `atomr_agents._native.workflow`      — workflow data types
//! - `atomr_agents._native.harness`       — `HarnessSpec`
//! - `atomr_agents._native.eval`          — eval suite + scorers
//! - `atomr_agents._native.guest`         — `register_*_factory`
//!                                          functions

#![allow(non_local_definitions)] // pyo3 macros emit local impls in modules.

use pyo3::prelude::*;

mod agent;
#[cfg(feature = "avatar")]
mod avatar;
mod cache;
mod callable;
mod channel;
mod coding_cli;
mod context;
mod conv;
mod core;
mod embed;
mod errors;
mod eval;
mod guest;
mod harness;
mod harness_adapters;
mod host;
mod inference;
mod ingest;
mod instruction;
mod memory;
mod observability;
mod org;
mod parser;
mod persona;
mod registry;
mod retriever;
mod runtime;
mod skill;
mod state;
mod strategy;
mod stt;
mod stt_harness;
mod tool;
mod tts;
mod voice;
mod voice_extras;
mod workflow;

/// Module init. Exposed as `atomr_agents._native`.
/// The function name matches the `[lib].name` in Cargo.toml, which is
/// what pyo3 uses to derive the `PyInit_...` symbol that CPython
/// looks up on import.
#[pymodule]
fn _native(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    errors::register(py, m)?;
    core::register(py, m)?;
    callable::register(py, m)?;
    strategy::register(py, m)?;
    embed::register(py, m)?;
    memory::register(py, m)?;
    instruction::register(py, m)?;
    context::register(py, m)?;
    state::register(py, m)?;
    observability::register(py, m)?;
    org::register(py, m)?;
    registry::register(py, m)?;
    retriever::register(py, m)?;
    ingest::register(py, m)?;
    tool::register(py, m)?;
    skill::register(py, m)?;
    persona::register(py, m)?;
    parser::register(py, m)?;
    cache::register(py, m)?;
    agent::register(py, m)?;
    workflow::register(py, m)?;
    harness::register(py, m)?;
    eval::register(py, m)?;
    host::register(py, m)?;
    guest::register(py, m)?;
    // PyHarness + loop_strategy / termination guest registration helpers
    // depend on `harness::register` and `guest::register` having created
    // their submodules first; attach into them.
    harness_adapters::register_into(py, m)?;
    stt::register(py, m)?;
    stt_harness::register(py, m)?;
    coding_cli::register(py, m)?;
    channel::register(py, m)?;
    tts::register(py, m)?;
    voice::register(py, m)?;
    voice_extras::register(py, m)?;
    #[cfg(feature = "avatar")]
    avatar::register(py, m)?;
    Ok(())
}
