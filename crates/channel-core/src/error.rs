use thiserror::Error;

#[derive(Debug, Error)]
pub enum ChannelError {
    #[error("unknown channel: {0}")]
    UnknownChannel(String),

    #[error("unknown thread: {0}")]
    UnknownThread(String),

    #[error("duplicate channel: {0}")]
    DuplicateChannel(String),

    #[error("provider error: {0}")]
    Provider(String),

    #[error("webhook verification failed: {0}")]
    WebhookVerify(String),

    #[error("webhook parse failed: {0}")]
    WebhookParse(String),

    #[error("capability disabled on this channel: {0}")]
    CapabilityDenied(&'static str),

    #[error("unsupported operation for this provider: {0}")]
    Unsupported(&'static str),

    #[error("config error: {0}")]
    Config(String),

    #[error("store error: {0}")]
    Store(String),

    #[error("transport error: {0}")]
    Transport(String),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error(transparent)]
    Agent(#[from] atomr_agents_core::AgentError),

    #[error("internal: {0}")]
    Internal(String),
}

impl ChannelError {
    pub fn provider(msg: impl Into<String>) -> Self {
        Self::Provider(msg.into())
    }

    pub fn webhook_verify(msg: impl Into<String>) -> Self {
        Self::WebhookVerify(msg.into())
    }

    pub fn webhook_parse(msg: impl Into<String>) -> Self {
        Self::WebhookParse(msg.into())
    }

    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }

    pub fn store(msg: impl Into<String>) -> Self {
        Self::Store(msg.into())
    }

    pub fn transport(msg: impl Into<String>) -> Self {
        Self::Transport(msg.into())
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }
}

pub type Result<T, E = ChannelError> = std::result::Result<T, E>;

impl From<ChannelError> for atomr_agents_core::AgentError {
    fn from(value: ChannelError) -> Self {
        match value {
            ChannelError::Agent(e) => e,
            other => atomr_agents_core::AgentError::Internal(other.to_string()),
        }
    }
}
