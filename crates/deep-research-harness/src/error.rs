//! Crate error type.

use atomr_agents_core::AgentError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DeepResearchError {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("role error: {0}")]
    Role(String),
    #[error("tool error: {0}")]
    Tool(String),
    #[error("web search error: {0}")]
    WebSearch(#[from] atomr_agents_web_search_core::WebSearchError),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("persistence error: {0}")]
    Persistence(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl DeepResearchError {
    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }
    pub fn role(msg: impl Into<String>) -> Self {
        Self::Role(msg.into())
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

impl From<DeepResearchError> for AgentError {
    fn from(e: DeepResearchError) -> Self {
        AgentError::Harness(e.to_string())
    }
}

pub type Result<T, E = DeepResearchError> = std::result::Result<T, E>;
