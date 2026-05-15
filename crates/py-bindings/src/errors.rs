//! Python exception hierarchy mirroring `atomr_agents_core::AgentError`
//! and the per-subsystem error variants surfaced by the wrapped Rust
//! crates. Layout mirrors `atomr-infer/inference-py-bindings/errors.rs`.
//!
//! Hierarchy (Python side):
//!
//! ```text
//! AgentError
//!  ├─ RegistryError
//!  ├─ BudgetExhausted
//!  ├─ ToolError
//!  ├─ StrategyError
//!  ├─ WorkflowError
//!  ├─ HarnessError
//!  ├─ EvalError
//!  ├─ MemoryError
//!  ├─ ParserError
//!  └─ CacheError
//! ```
//!
//! Rust callers should funnel `?` through `crate::errors::map(e)` so
//! the Python side sees consistent exception types.

use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;

create_exception!(atomr_agents, AgentError, PyException, "Base atomr-agents error.");
create_exception!(
    atomr_agents,
    RegistryError,
    AgentError,
    "Artifact registry lookup, publish, or eval-gate failure."
);
create_exception!(
    atomr_agents,
    BudgetExhausted,
    AgentError,
    "Token / time / money / iteration budget exceeded."
);
create_exception!(
    atomr_agents,
    ToolError,
    AgentError,
    "Tool invocation, descriptor, or registry failure."
);
create_exception!(
    atomr_agents,
    StrategyError,
    AgentError,
    "Strategy resolution failure."
);
create_exception!(
    atomr_agents,
    WorkflowError,
    AgentError,
    "Workflow / DAG execution failure."
);
create_exception!(
    atomr_agents,
    HarnessError,
    AgentError,
    "Persistent harness loop failure."
);
create_exception!(
    atomr_agents,
    EvalError,
    AgentError,
    "Eval suite, scorer, or regression-gate failure."
);
create_exception!(
    atomr_agents,
    MemoryError,
    AgentError,
    "Memory store / long store / namespace failure."
);
create_exception!(
    atomr_agents,
    ParserError,
    AgentError,
    "Output parser / schema validation failure."
);
create_exception!(
    atomr_agents,
    CacheError,
    AgentError,
    "LLM cache get / put failure."
);

/// Map any error type that implements `Display` onto the base
/// `AgentError`. Per-subsystem callers can raise more specific
/// variants by constructing them directly:
///
/// ```ignore
/// return Err(PyErr::new::<crate::errors::ToolError, _>(format!("…")));
/// ```
pub fn map<E: std::fmt::Display>(e: E) -> PyErr {
    PyErr::new::<AgentError, _>(e.to_string())
}

/// Convenience: map a `Result<T, E: Display>` into a `PyResult<T>`.
pub fn into_py<T, E: std::fmt::Display>(r: Result<T, E>) -> PyResult<T> {
    r.map_err(map)
}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "errors")?;
    m.add("AgentError", py.get_type_bound::<AgentError>())?;
    m.add("RegistryError", py.get_type_bound::<RegistryError>())?;
    m.add("BudgetExhausted", py.get_type_bound::<BudgetExhausted>())?;
    m.add("ToolError", py.get_type_bound::<ToolError>())?;
    m.add("StrategyError", py.get_type_bound::<StrategyError>())?;
    m.add("WorkflowError", py.get_type_bound::<WorkflowError>())?;
    m.add("HarnessError", py.get_type_bound::<HarnessError>())?;
    m.add("EvalError", py.get_type_bound::<EvalError>())?;
    m.add("MemoryError", py.get_type_bound::<MemoryError>())?;
    m.add("ParserError", py.get_type_bound::<ParserError>())?;
    m.add("CacheError", py.get_type_bound::<CacheError>())?;
    parent.add_submodule(&m)?;
    Ok(())
}
