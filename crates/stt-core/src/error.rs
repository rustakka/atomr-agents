//! STT-specific error type. Mirrors the shape of `AgentError` in
//! `atomr-agents-core` but with audio/transport-specific variants.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SttError {
    #[error("audio decode failed: {0}")]
    Decode(String),

    #[error("unsupported audio format: {0}")]
    UnsupportedFormat(String),

    #[error("backend rejected request ({status}): {message}")]
    Backend { status: u16, message: String },

    #[error("transport error: {0}")]
    Transport(String),

    #[error("authentication failed")]
    Auth,

    #[error("rate limited; retry after {retry_after_ms} ms")]
    RateLimited { retry_after_ms: u64 },

    #[error("capability not supported by backend: {0}")]
    UnsupportedCapability(&'static str),

    #[error("model load failed: {0}")]
    ModelLoad(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("internal: {0}")]
    Internal(String),
}

impl SttError {
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }

    pub fn decode(msg: impl Into<String>) -> Self {
        Self::Decode(msg.into())
    }

    pub fn transport(msg: impl Into<String>) -> Self {
        Self::Transport(msg.into())
    }

    pub fn model_load(msg: impl Into<String>) -> Self {
        Self::ModelLoad(msg.into())
    }
}

pub type Result<T, E = SttError> = std::result::Result<T, E>;
