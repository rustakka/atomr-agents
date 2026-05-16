//! Avatar-domain error type.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AvatarError {
    #[error("config error: {0}")]
    Config(String),

    #[error("sink error: {0}")]
    Sink(String),

    #[error("transport error: {0}")]
    Transport(String),

    #[error("encode error: {0}")]
    Encode(String),

    #[error("decode error: {0}")]
    Decode(String),

    #[error("unsupported operation: {0}")]
    Unsupported(String),

    #[error("blendshape weight out of range [0.0, 1.0]: {value} for {name}")]
    BlendshapeRange { name: &'static str, value: f32 },

    #[error("blendshape vector wrong length: expected {expected}, got {got}")]
    BlendshapeLength { expected: usize, got: usize },

    #[error("perception error: {0}")]
    Perception(String),

    #[error("cognition error: {0}")]
    Cognition(String),

    #[error("synthesis error: {0}")]
    Synthesis(String),

    #[error("sync error: {0}")]
    Sync(String),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error(transparent)]
    Agent(#[from] atomr_agents_core::AgentError),

    #[error("internal: {0}")]
    Internal(String),
}

impl AvatarError {
    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }
    pub fn sink(msg: impl Into<String>) -> Self {
        Self::Sink(msg.into())
    }
    pub fn transport(msg: impl Into<String>) -> Self {
        Self::Transport(msg.into())
    }
    pub fn encode(msg: impl Into<String>) -> Self {
        Self::Encode(msg.into())
    }
    pub fn decode(msg: impl Into<String>) -> Self {
        Self::Decode(msg.into())
    }
    pub fn unsupported(msg: impl Into<String>) -> Self {
        Self::Unsupported(msg.into())
    }
    pub fn perception(msg: impl Into<String>) -> Self {
        Self::Perception(msg.into())
    }
    pub fn cognition(msg: impl Into<String>) -> Self {
        Self::Cognition(msg.into())
    }
    pub fn synthesis(msg: impl Into<String>) -> Self {
        Self::Synthesis(msg.into())
    }
    pub fn sync(msg: impl Into<String>) -> Self {
        Self::Sync(msg.into())
    }
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }
}

pub type Result<T, E = AvatarError> = std::result::Result<T, E>;

impl From<AvatarError> for atomr_agents_core::AgentError {
    fn from(value: AvatarError) -> Self {
        match value {
            AvatarError::Agent(e) => e,
            other => atomr_agents_core::AgentError::Internal(other.to_string()),
        }
    }
}
