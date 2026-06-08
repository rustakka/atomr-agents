//! Errors raised by an [`AgentSdkBackend`](crate::AgentSdkBackend).
//!
//! Kept atomr-core-free (like `coding-cli-core`): the harness crate owns a
//! local `HarnessError` that wraps this and converts to
//! `atomr_agents_core::AgentError` (the orphan rule requires the local
//! type to live there, not here).

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentSdkError {
    /// The backend is not usable on this host (SDK not installed, `claude`
    /// CLI missing, credentials absent…).
    #[error("agent-sdk backend unavailable: {0}")]
    Unavailable(String),

    /// The request/config could not be built into valid options.
    #[error("invalid agent-sdk config: {0}")]
    InvalidConfig(String),

    /// Referenced an interactive session that does not exist.
    #[error("agent-sdk session not found: {0}")]
    SessionNotFound(String),

    /// Concurrency quota for interactive sessions reached.
    #[error("agent-sdk session quota reached ({0})")]
    SessionQuota(usize),

    /// A configured budget cap was exceeded mid-run.
    #[error("agent-sdk budget exhausted: {0}")]
    BudgetExhausted(&'static str),

    /// A tool was denied by the configured permission policy.
    #[error("agent-sdk policy denied: {0}")]
    PolicyDenied(String),

    /// The underlying SDK / `claude` CLI raised an error.
    #[error("agent-sdk error: {0}")]
    Sdk(String),

    /// The message stream closed before yielding a terminal result.
    #[error("agent-sdk stream closed before result")]
    StreamClosed,

    /// The run was cancelled / interrupted.
    #[error("agent-sdk run cancelled")]
    Cancelled,

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}
