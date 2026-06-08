//! [`AgentSdkConfig`] — a serde mirror of the SDK's `ClaudeAgentOptions`.
//!
//! `serde_json::to_value(&AgentSdkConfig)` produces a dict the Python
//! wrapper turns back into `ClaudeAgentOptions(**dict)` (after popping the
//! fields that carry live callbacks, which can't cross JSON — see below).
//!
//! **Callbacks are carried as data descriptors only.** `can_use_tool`,
//! hooks, in-process MCP servers, and subagents reference Python closures
//! that cannot serialize. The config carries [`PermissionPolicy`],
//! [`HookHandlerRef`], [`McpServerConfig::InProcess`], and [`SubagentDef`]
//! as plain data; the Python wrapper resolves them to live closures
//! host-side. This keeps `AgentSdkConfig` a pure round-tripping dict.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Mirror of `ClaudeAgentOptions`. Every field is optional so a minimal
/// `{"prompt": "..."}` request deserializes; omitted fields fall back to
/// the SDK's own defaults on the Python side.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentSdkConfig {
    /// Custom system prompt: a bare string, or the `claude_code` preset
    /// (optionally appended to).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<SystemPromptConfig>,

    /// Tools auto-approved without a permission prompt.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,

    /// Tools removed from the available set entirely.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disallowed_tools: Vec<String>,

    /// Claude Code's tool-approval behavior. Defaults to
    /// [`PermissionMode::BypassPermissions`] (max autonomy — override per
    /// run/spec; see the harness safety note).
    #[serde(default)]
    pub permission_mode: PermissionMode,

    /// Working directory the agent operates in. Required at run time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,

    /// Additional directories granted filesystem access beyond `cwd`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_dirs: Vec<PathBuf>,

    /// Environment variables passed to the `claude` subprocess.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,

    /// MCP servers (external stdio/SSE/HTTP, or an in-process marker the
    /// wrapper resolves to an SDK MCP server).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub mcp_servers: BTreeMap<String, McpServerConfig>,

    /// Lifecycle hooks (data descriptors; live callbacks injected host-side).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hooks: Vec<HookConfig>,

    /// Programmatically-defined subagents, keyed by name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub agents: BTreeMap<String, SubagentDef>,

    /// Which filesystem settings to load (controls `.claude/` config and
    /// slash-command / `CLAUDE.md` discovery). Defaults to `["project"]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub setting_sources: Vec<SettingSource>,

    /// Model id (alias like `sonnet`/`opus` or a full id). When unset, the
    /// harness applies its spec default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Fallback model the SDK uses if the primary is overloaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_model: Option<String>,

    /// Cap on agent turns (SDK-enforced hard limit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,

    /// Resume most-recent session in `cwd`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub continue_conversation: bool,

    /// Resume a specific session by id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<String>,

    /// Fork the resumed session into a fresh one with a copy of history.
    #[serde(default, skip_serializing_if = "is_false")]
    pub fork_session: bool,

    /// Emit partial (streaming-delta) assistant messages.
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_partial_messages: bool,

    /// Reasoning-effort level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<Effort>,

    /// Extended-thinking configuration (opaque pass-through to the SDK).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<serde_json::Value>,

    /// Data form of the SDK's `can_use_tool` callback. The wrapper turns
    /// this into a live permission callback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_policy: Option<PermissionPolicy>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Claude Code's tool-approval behavior. Serializes to the SDK's camelCase
/// strings (`default`, `acceptEdits`, `plan`, `bypassPermissions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    Default,
    AcceptEdits,
    Plan,
    /// Auto-approve every tool. The harness default (max autonomy).
    #[default]
    BypassPermissions,
}

impl PermissionMode {
    /// The wire string the SDK expects.
    pub fn as_sdk_str(&self) -> &'static str {
        match self {
            PermissionMode::Default => "default",
            PermissionMode::AcceptEdits => "acceptEdits",
            PermissionMode::Plan => "plan",
            PermissionMode::BypassPermissions => "bypassPermissions",
        }
    }
}

/// System prompt: a bare string, or the `claude_code` preset.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SystemPromptConfig {
    Text(String),
    Preset(SystemPromptPreset),
}

/// The `{"type":"preset","preset":"claude_code","append":...}` shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemPromptPreset {
    #[serde(rename = "type")]
    pub kind: String,
    pub preset: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub append: Option<String>,
}

impl SystemPromptPreset {
    /// The default `claude_code` preset, optionally appended to.
    pub fn claude_code(append: Option<String>) -> Self {
        Self {
            kind: "preset".into(),
            preset: "claude_code".into(),
            append,
        }
    }
}

/// One external or in-process MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "snake_case")]
pub enum McpServerConfig {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    Sse {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
    Http {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
    /// Marker the wrapper resolves to an SDK in-process MCP server (the
    /// atomr tool bridge).
    InProcess { name: String },
}

/// A lifecycle hook descriptor. The `handler` names a callback the wrapper
/// resolves to a live Python hook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfig {
    pub event: HookEvent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    pub handler: HookHandlerRef,
}

/// Named reference to a host-registered hook callback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookHandlerRef {
    pub name: String,
}

/// SDK hook events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    UserPromptSubmit,
    Stop,
    SubagentStop,
    PreCompact,
}

/// A programmatically-defined subagent (maps to the SDK's `AgentDefinition`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentDef {
    pub description: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Which filesystem settings the SDK loads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingSource {
    User,
    Project,
    Local,
}

/// Reasoning-effort level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

/// Data form of the SDK's `can_use_tool` callback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum PermissionPolicy {
    /// Allow every tool (the wrapper returns "allow" unconditionally).
    #[default]
    AllowAll,
    /// Deny every tool.
    DenyAll,
    /// Allow only the listed tools, deny the rest.
    AllowList { tools: Vec<String> },
    /// Defer to the SDK's interactive ask flow.
    Ask,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_mode_default_is_bypass() {
        assert_eq!(PermissionMode::default(), PermissionMode::BypassPermissions);
        let j = serde_json::to_string(&PermissionMode::AcceptEdits).unwrap();
        assert_eq!(j, "\"acceptEdits\"");
    }

    #[test]
    fn minimal_config_round_trips() {
        let cfg: AgentSdkConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.permission_mode, PermissionMode::BypassPermissions);
        assert!(cfg.allowed_tools.is_empty());
        let back = serde_json::to_value(&cfg).unwrap();
        // Empty collections + Nones are skipped; default permission mode stays.
        assert_eq!(back["permission_mode"], "bypassPermissions");
    }

    #[test]
    fn system_prompt_preset_shape() {
        let p = SystemPromptConfig::Preset(SystemPromptPreset::claude_code(Some("extra".into())));
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["type"], "preset");
        assert_eq!(v["preset"], "claude_code");
        assert_eq!(v["append"], "extra");
    }

    #[test]
    fn system_prompt_text_is_bare_string() {
        let p = SystemPromptConfig::Text("hi".into());
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v, serde_json::json!("hi"));
    }

    #[test]
    fn setting_source_lowercases() {
        let j = serde_json::to_string(&SettingSource::Project).unwrap();
        assert_eq!(j, "\"project\"");
    }
}
