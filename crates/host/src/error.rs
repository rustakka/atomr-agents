//! Host-wide error type.

use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum HostError {
    #[error("agent `{0}` not found at {1}")]
    AgentNotFound(String, PathBuf),

    #[error("agent spec error: {0}")]
    AgentSpec(String),

    #[error("host config error: {0}")]
    Config(String),

    #[error("io error at {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("yaml error at {path:?}: {source}")]
    Yaml {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },

    #[error("json error at {path:?}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("invalid markdown at {path:?}: {reason}")]
    Markdown { path: PathBuf, reason: String },

    #[error("invalid skill `{id}`: {reason}")]
    Skill { id: String, reason: String },

    #[error("invalid hook at {path:?}: {reason}")]
    Hook { path: PathBuf, reason: String },

    #[error("actor system error: {0}")]
    ActorSystem(String),

    #[error("scheduler error: {0}")]
    Scheduler(String),

    #[error("registry error: {0}")]
    Registry(String),

    #[error("eval error: {0}")]
    Eval(String),

    #[error("branching error: {0}")]
    Branching(String),

    #[error("mcp error: {0}")]
    Mcp(String),

    #[error("gateway error: {0}")]
    Gateway(String),

    #[error("curator error: {0}")]
    Curator(String),

    #[error("hook dispatch error: {0}")]
    HookDispatch(String),

    #[error("other: {0}")]
    Other(#[from] anyhow::Error),
}

impl HostError {
    pub fn agent_spec(msg: impl Into<String>) -> Self {
        Self::AgentSpec(msg.into())
    }

    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }

    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io { path: path.into(), source }
    }

    pub fn yaml(path: impl Into<PathBuf>, source: serde_yaml::Error) -> Self {
        Self::Yaml { path: path.into(), source }
    }

    pub fn json(path: impl Into<PathBuf>, source: serde_json::Error) -> Self {
        Self::Json { path: path.into(), source }
    }

    pub fn markdown(path: impl Into<PathBuf>, reason: impl Into<String>) -> Self {
        Self::Markdown { path: path.into(), reason: reason.into() }
    }

    pub fn skill(id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Skill { id: id.into(), reason: reason.into() }
    }

    pub fn hook(path: impl Into<PathBuf>, reason: impl Into<String>) -> Self {
        Self::Hook { path: path.into(), reason: reason.into() }
    }
}

pub type HostResult<T> = Result<T, HostError>;
