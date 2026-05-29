use atomr_agents_core::AgentError;
use thiserror::Error;

/// Errors raised by the security substrate. These map into the
/// framework-wide [`AgentError::PolicyDenied`] when surfaced through a
/// `Tool`/`Callable` boundary, but callers of the broker / wall APIs
/// can match the typed variant directly.
#[derive(Debug, Error)]
pub enum SecurityError {
    /// Clearance check failed (missing context, insufficient level, or
    /// a compartment the subject does not hold). Fail-closed.
    #[error("access denied: {0}")]
    AccessDenied(String),

    /// A mandate / pre-trade boundary rejected the call.
    #[error("mandate violation: {0}")]
    MandateViolation(String),

    /// A capability handle could not be resolved (unknown handle).
    #[error("unknown capability handle: {0}")]
    UnknownCapability(String),
}

impl From<SecurityError> for AgentError {
    fn from(e: SecurityError) -> Self {
        AgentError::PolicyDenied(e.to_string())
    }
}
