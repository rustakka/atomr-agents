use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("budget exceeded: {0}")]
    BudgetExceeded(&'static str),

    #[error("strategy resolution failed: {0}")]
    Strategy(String),

    #[error("tool invocation failed: {0}")]
    Tool(String),

    #[error("memory operation failed: {0}")]
    Memory(String),

    #[error("inference call failed: {0}")]
    Inference(String),

    #[error("policy denied: {0}")]
    PolicyDenied(String),

    #[error("workflow error: {0}")]
    Workflow(String),

    #[error("harness error: {0}")]
    Harness(String),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("internal: {0}")]
    Internal(String),
}

impl AgentError {
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }
}

pub type Result<T, E = AgentError> = std::result::Result<T, E>;
