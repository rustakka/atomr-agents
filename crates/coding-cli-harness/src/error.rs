use thiserror::Error;

use atomr_agents_coding_cli_core::{CliVendorKind, MapperError, ParseError};
use atomr_agents_coding_cli_isolator::IsolatorError;

#[derive(Debug, Error)]
pub enum HarnessError {
    #[error("unknown vendor: {0}")]
    UnknownVendor(CliVendorKind),

    #[error("vendor not available locally: {0}")]
    VendorUnavailable(CliVendorKind),

    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("workdir is missing or not a directory: {0}")]
    InvalidWorkdir(String),

    #[error("session not found: {0}")]
    SessionNotFound(String),

    #[error(transparent)]
    Mapper(#[from] MapperError),

    #[error(transparent)]
    Isolator(#[from] IsolatorError),

    #[error("parse error: {0}")]
    Parse(#[from] ParseError),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("cancelled")]
    Cancelled,
}

pub type Result<T, E = HarnessError> = std::result::Result<T, E>;

impl From<HarnessError> for atomr_agents_core::AgentError {
    fn from(e: HarnessError) -> Self {
        atomr_agents_core::AgentError::Harness(e.to_string())
    }
}
