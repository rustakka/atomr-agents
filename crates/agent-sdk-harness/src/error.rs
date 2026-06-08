use thiserror::Error;

use atomr_agents_agent_sdk_core::AgentSdkError;

#[derive(Debug, Error)]
pub enum HarnessError {
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("workdir is missing or not a directory: {0}")]
    InvalidWorkdir(String),

    #[error("session not found: {0}")]
    SessionNotFound(String),

    #[error("session quota reached ({0})")]
    SessionQuota(usize),

    #[error("budget exhausted: {0}")]
    Budget(&'static str),

    #[error("policy denied: {0}")]
    PolicyDenied(String),

    #[error("stream closed before result")]
    StreamClosed,

    #[error(transparent)]
    Sdk(#[from] AgentSdkError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

pub type Result<T, E = HarnessError> = std::result::Result<T, E>;

impl From<HarnessError> for atomr_agents_core::AgentError {
    fn from(e: HarnessError) -> Self {
        use atomr_agents_core::AgentError as A;
        let msg = e.to_string();
        match e {
            HarnessError::Budget(s) => A::BudgetExceeded(s),
            HarnessError::PolicyDenied(s) => A::PolicyDenied(s),
            HarnessError::Sdk(AgentSdkError::PolicyDenied(s)) => A::PolicyDenied(s),
            HarnessError::Sdk(AgentSdkError::BudgetExhausted(s)) => A::BudgetExceeded(s),
            HarnessError::Sdk(_) => A::Inference(msg),
            _ => A::Harness(msg),
        }
    }
}
