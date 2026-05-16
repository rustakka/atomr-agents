//! Crate error type.

use atomr_agents_core::AgentError;
use thiserror::Error;

/// Errors raised by the deep-research shell.
#[derive(Debug, Error)]
pub enum ShellError {
    /// Misconfiguration (e.g. no classifier/shallow/deep wired).
    #[error("configuration error: {0}")]
    Config(String),

    /// The intent classifier failed.
    #[error("classifier error: {0}")]
    Classifier(String),

    /// The shallow-path researcher failed.
    #[error("shallow path error: {0}")]
    Shallow(String),

    /// The deep harness returned an error.
    #[error("deep harness error: {0}")]
    Deep(String),

    /// Underlying web-search provider failure.
    #[error("web search error: {0}")]
    WebSearch(#[from] atomr_agents_web_search_core::WebSearchError),

    /// Serialization / deserialization failure.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// Catch-all.
    #[error("internal error: {0}")]
    Internal(String),
}

impl ShellError {
    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }
    pub fn classifier(msg: impl Into<String>) -> Self {
        Self::Classifier(msg.into())
    }
    pub fn shallow(msg: impl Into<String>) -> Self {
        Self::Shallow(msg.into())
    }
    pub fn deep(msg: impl Into<String>) -> Self {
        Self::Deep(msg.into())
    }
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }
}

impl From<ShellError> for AgentError {
    fn from(e: ShellError) -> Self {
        AgentError::Harness(e.to_string())
    }
}

/// Crate result alias.
pub type Result<T, E = ShellError> = std::result::Result<T, E>;
