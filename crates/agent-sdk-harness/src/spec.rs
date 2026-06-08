//! Top-level harness configuration.

use serde::{Deserialize, Serialize};

use atomr_agents_agent_sdk_core::{Effort, PermissionMode, SettingSource};

/// How the harness selects Anthropic credentials at spawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AuthProvider {
    /// `ANTHROPIC_API_KEY` (first-party / Claude Platform on AWS).
    #[default]
    Anthropic,
    /// `CLAUDE_CODE_USE_BEDROCK=1` + AWS credentials.
    Bedrock,
    /// `CLAUDE_CODE_USE_VERTEX=1` + GCP credentials.
    Vertex,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthConfig {
    #[serde(default)]
    pub provider: AuthProvider,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSdkHarnessSpec {
    /// Stable harness id (used in telemetry / `Event::HarnessIteration`).
    #[serde(default = "default_id")]
    pub id: String,

    /// Model used when a request doesn't name one.
    #[serde(default = "default_model")]
    pub default_model: Option<String>,

    /// Fallback model if the primary is overloaded.
    #[serde(default)]
    pub fallback_model: Option<String>,

    /// Default tool-approval behavior. **`bypassPermissions`** (max
    /// autonomy) — override per request/spec; see the safety note in the
    /// crate docs.
    #[serde(default)]
    pub default_permission_mode: PermissionMode,

    /// Which filesystem settings the SDK loads. Defaults to `["project"]`
    /// so the agent sees only harness-materialized `.claude/` config.
    #[serde(default = "default_setting_sources")]
    pub default_setting_sources: Vec<SettingSource>,

    /// Default reasoning effort.
    #[serde(default)]
    pub default_effort: Option<Effort>,

    /// Cap on simultaneous interactive sessions.
    #[serde(default = "default_max_sessions")]
    pub max_concurrent_sessions: usize,

    /// Capacity of the broadcast event channel.
    #[serde(default = "default_channel_cap")]
    pub event_channel_capacity: usize,

    /// Default turn cap (SDK-enforced).
    #[serde(default)]
    pub default_max_turns: Option<u32>,

    /// Default soft cost cap (USD) enforced at turn boundaries.
    #[serde(default)]
    pub default_max_cost_usd: Option<f64>,

    /// Credential selection.
    #[serde(default)]
    pub auth: AuthConfig,
}

impl Default for AgentSdkHarnessSpec {
    fn default() -> Self {
        Self {
            id: default_id(),
            default_model: default_model(),
            fallback_model: None,
            default_permission_mode: PermissionMode::default(),
            default_setting_sources: default_setting_sources(),
            default_effort: None,
            max_concurrent_sessions: default_max_sessions(),
            event_channel_capacity: default_channel_cap(),
            default_max_turns: None,
            default_max_cost_usd: None,
            auth: AuthConfig::default(),
        }
    }
}

fn default_id() -> String {
    "agent-sdk".to_string()
}
fn default_model() -> Option<String> {
    Some("claude-opus-4-8".to_string())
}
fn default_setting_sources() -> Vec<SettingSource> {
    vec![SettingSource::Project]
}
fn default_max_sessions() -> usize {
    16
}
fn default_channel_cap() -> usize {
    512
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_spec_is_bypass_and_project() {
        let s = AgentSdkHarnessSpec::default();
        assert_eq!(s.default_permission_mode, PermissionMode::BypassPermissions);
        assert_eq!(s.default_setting_sources, vec![SettingSource::Project]);
        assert_eq!(s.default_model.as_deref(), Some("claude-opus-4-8"));
        assert_eq!(s.auth.provider, AuthProvider::Anthropic);
    }

    #[test]
    fn spec_round_trips_yaml_like_json() {
        let s: AgentSdkHarnessSpec =
            serde_json::from_str(r#"{"id":"reviewer","default_model":"claude-sonnet-4-6"}"#).unwrap();
        assert_eq!(s.id, "reviewer");
        assert_eq!(s.default_model.as_deref(), Some("claude-sonnet-4-6"));
        // Unspecified fields fall back to defaults.
        assert_eq!(s.max_concurrent_sessions, 16);
    }
}
