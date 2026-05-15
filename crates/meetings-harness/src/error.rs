//! Crate error type.

use atomr_agents_core::AgentError;
use atomr_agents_stt_harness::SttHarnessError;
use thiserror::Error;

/// Anything that can go wrong building or driving a meetings harness.
#[derive(Debug, Error)]
pub enum MeetingsHarnessError {
    /// The source STT conversation could not be loaded from the
    /// configured store.
    #[error("source transcript not found: {0}")]
    TranscriptNotFound(String),

    /// Misconfiguration (missing model id, conflicting modes, etc.).
    #[error("configuration error: {0}")]
    Config(String),

    /// The extractor returned an error.
    #[error("extraction error: {0}")]
    Extraction(String),

    /// A tool invocation failed (bad args, broken invariant).
    #[error("tool error: {0}")]
    Tool(String),

    /// JSON (de)serialization failed.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// Underlying STT-harness error (transcript load, persistence).
    #[error("stt-harness error: {0}")]
    Stt(#[from] SttHarnessError),

    /// A persistence backend rejected an operation.
    #[error("persistence error: {0}")]
    Persistence(String),

    /// Catch-all for internal invariants.
    #[error("internal error: {0}")]
    Internal(String),
}

impl MeetingsHarnessError {
    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }
    pub fn extraction(msg: impl Into<String>) -> Self {
        Self::Extraction(msg.into())
    }
    pub fn tool(msg: impl Into<String>) -> Self {
        Self::Tool(msg.into())
    }
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }
    pub fn persistence(msg: impl Into<String>) -> Self {
        Self::Persistence(msg.into())
    }
}

impl From<MeetingsHarnessError> for AgentError {
    fn from(e: MeetingsHarnessError) -> Self {
        AgentError::Harness(e.to_string())
    }
}

pub type Result<T, E = MeetingsHarnessError> = std::result::Result<T, E>;
