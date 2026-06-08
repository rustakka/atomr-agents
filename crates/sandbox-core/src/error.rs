//! Sandbox error taxonomy. Mirrors the shape of `IsolatorError` in
//! `coding-cli-isolator` so backend implementations translate uniformly.

use thiserror::Error;

use crate::id::SandboxId;

#[derive(Debug, Error)]
pub enum SandboxError {
    /// The backend does not implement this capability (e.g. snapshot/fork
    /// on the Docker "insecure dev mode" backend).
    #[error("not supported by this backend: {0}")]
    Unsupported(&'static str),

    /// A backend-internal failure (hypervisor API, container daemon, gRPC).
    #[error("backend error: {0}")]
    Backend(String),

    /// The operation exceeded its wall-clock budget.
    #[error("operation timed out")]
    Timeout,

    /// No live sandbox with the given id.
    #[error("sandbox not found: {0}")]
    NotFound(SandboxId),

    /// A request violated a resource / policy budget (e.g. unsupported
    /// language for the chosen profile, oversize file write).
    #[error("budget violation: {0}")]
    BudgetViolation(String),

    /// The in-guest agent reported an error over the vsock channel.
    #[error("guest agent error: {0}")]
    Guest(String),

    /// Host I/O failure.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T, E = SandboxError> = std::result::Result<T, E>;
