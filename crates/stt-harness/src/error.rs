//! Crate error type.
//!
//! `SttHarnessError` wraps the STT-stack error ([`SttError`]) plus the
//! harness-specific failure modes (audio setup, configuration,
//! serialization). It converts into [`atomr_agents_core::AgentError`] so
//! an [`crate::SttHarnessRef`] can satisfy the `Callable` contract.

use atomr_agents_core::AgentError;
use atomr_agents_stt_core::SttError;
use thiserror::Error;

/// Anything that can go wrong while building or driving an STT harness.
#[derive(Debug, Error)]
pub enum SttHarnessError {
    /// An error surfaced by the STT backend, streaming session, or
    /// diarizer.
    #[error("stt error: {0}")]
    Stt(#[from] SttError),

    /// The audio source could not be opened or decoded (missing
    /// feature, unreadable file, no input device).
    #[error("audio error: {0}")]
    Audio(String),

    /// The harness was misconfigured (e.g. the audio source was
    /// already consumed by a prior `run()`).
    #[error("configuration error: {0}")]
    Config(String),

    /// JSON (de)serialization of a conversation failed.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// A persistence backend (`crates/state`) rejected an operation.
    #[error("persistence error: {0}")]
    Persistence(String),

    /// Catch-all for internal invariants.
    #[error("internal error: {0}")]
    Internal(String),
}

impl SttHarnessError {
    pub fn audio(msg: impl Into<String>) -> Self {
        Self::Audio(msg.into())
    }

    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }
}

impl From<SttHarnessError> for AgentError {
    fn from(e: SttHarnessError) -> Self {
        AgentError::Harness(e.to_string())
    }
}

/// Crate result alias. Defaults to [`SttHarnessError`]; the `Callable`
/// surface re-maps into `atomr_agents_core::Result`.
pub type Result<T, E = SttHarnessError> = std::result::Result<T, E>;
