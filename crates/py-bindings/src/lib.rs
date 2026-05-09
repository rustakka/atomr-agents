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
mod cache;
mod context;
mod conv;
mod core;
mod errors;
mod eval;
mod guest;
mod harness;
mod observability;
mod parser;
mod persona;
mod registry;
mod runtime;
mod skill;
mod state;
mod tool;
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
    context::register(py, m)?;
    state::register(py, m)?;
    observability::register(py, m)?;
    registry::register(py, m)?;
    tool::register(py, m)?;
    skill::register(py, m)?;
    persona::register(py, m)?;
    parser::register(py, m)?;
    cache::register(py, m)?;
    agent::register(py, m)?;
    workflow::register(py, m)?;
    harness::register(py, m)?;
    eval::register(py, m)?;
    guest::register(py, m)?;
    Ok(())
}
