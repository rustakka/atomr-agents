//! Inputs accepted by the agent-sdk harness.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::AgentSdkConfig;

macro_rules! id_newtype {
    ($name:ident, $prefix:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new() -> Self {
                Self(format!("{}-{}", $prefix, Uuid::new_v4()))
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }
        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_owned())
            }
        }
    };
}

id_newtype!(AgentRunId, "agent-run");
// `AgentSessionId` is an opaque handle to a live interactive session (the
// registry / actor key). Distinct from the SDK's *conversation* `session_id`
// (surfaced on messages/results for resume).
id_newtype!(AgentSessionId, "agent-sess");

/// A one-shot headless query. `config` is flattened so a request reads as
/// `{"prompt": "...", "allowed_tools": [...], ...}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryRequest {
    pub prompt: String,
    #[serde(flatten)]
    pub config: AgentSdkConfig,
    /// Soft cost cap (USD) enforced at turn boundaries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<f64>,
}

impl QueryRequest {
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            config: AgentSdkConfig::default(),
            max_cost_usd: None,
        }
    }

    pub fn with_config(mut self, config: AgentSdkConfig) -> Self {
        self.config = config;
        self
    }
}

/// Spec for a stateful interactive session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionSpec {
    #[serde(flatten)]
    pub config: AgentSdkConfig,
    /// Optional first prompt sent on connect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_prompt: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_flattens_config() {
        let j = r#"{"prompt":"hi","allowed_tools":["Read"],"model":"opus"}"#;
        let q: QueryRequest = serde_json::from_str(j).unwrap();
        assert_eq!(q.prompt, "hi");
        assert_eq!(q.config.allowed_tools, vec!["Read".to_string()]);
        assert_eq!(q.config.model.as_deref(), Some("opus"));
    }

    #[test]
    fn run_id_unique_and_prefixed() {
        let a = AgentRunId::new();
        let b = AgentRunId::new();
        assert_ne!(a, b);
        assert!(a.as_str().starts_with("agent-run-"));
    }
}
