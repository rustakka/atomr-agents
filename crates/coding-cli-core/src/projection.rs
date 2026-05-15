//! Flat, serde-friendly snapshots of atomr concepts a `CliVendor`
//! adapter projects onto its on-disk CLI config (`CLAUDE.md`,
//! `.cursor/rules/*`, `AGENTS.md`, `.mcp.json`, ...).
//!
//! Snapshots are *one-way*: the harness builds them from live
//! `atomr-agents-skill::Skill`, `atomr-agents-persona::Persona`,
//! `atomr-agents-strategy::Policy` values at run time and hands them
//! to the vendor. The vendor never mutates atomr state from disk.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Identity + behavior of the agent the CLI should impersonate.
///
/// Materialized into the vendor's "system instruction" surface:
/// the top of `CLAUDE.md`, the system-instruction file for Gemini,
/// the top of `AGENTS.md` for Codex.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersonaSnapshot {
    pub identity: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub salient_traits: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style_tone: Option<String>,
}

/// One skill the CLI should be able to invoke.
///
/// For Claude Code this becomes `~/.claude/skills/<id>/SKILL.md` (or
/// `<workdir>/.claude/skills/<id>/SKILL.md` for project scope).
/// For Cursor this maps to `.cursor/rules/<id>.mdc`. For Gemini and
/// Codex (which lack a native concept) skills are concatenated into
/// the system-instruction file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSnapshot {
    pub id: String,
    pub name: String,
    pub instruction_fragment: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default = "default_priority")]
    pub priority: u8,
    /// Tool names this skill expects to be available. Up to the vendor
    /// adapter how to surface this (Claude allows `--allowed-tools`;
    /// others may just list them in the instruction fragment).
    #[serde(default)]
    pub tools: Vec<String>,
}

fn default_priority() -> u8 {
    5
}

/// Narrowed permissions the harness wants enforced by the CLI itself.
///
/// Materialized as `--allowed-tools` / `--model` flags (Claude),
/// `--full-access` gating (Codex), settings.json permissions, etc.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PolicySnapshot {
    /// Tool name allow-list. Empty means "no override — let the CLI
    /// use its default policy".
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Model id allow-list. The harness picks the first that the
    /// `CliRequest::model` field intersects with.
    #[serde(default)]
    pub allowed_models: Vec<String>,
    /// If `true`, the adapter is permitted to pass its
    /// "auto-approve" flag (`--full-access`, `--yolo`, ...).
    #[serde(default)]
    pub auto_approve_unrestricted: bool,
    /// Per-call token cap (used as a hint; not every CLI enforces it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens_per_call: Option<u32>,
}

/// One MCP server entry — vendor-agnostic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerSnapshot {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

/// A bundled set of MCP servers + standalone tool names. Materializes
/// to `.mcp.json` (Claude), `.cursor/mcp.json` (Cursor), Codex /
/// Gemini settings files.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolSetSnapshot {
    pub id: String,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerSnapshot>,
    #[serde(default)]
    pub tool_names: Vec<String>,
}

/// Everything the harness hands a vendor adapter before a run so the
/// adapter can materialize on-disk config.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConceptProjection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona: Option<PersonaSnapshot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<SkillSnapshot>,
    #[serde(default)]
    pub policy: PolicySnapshot,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub toolsets: Vec<ToolSetSnapshot>,
    /// Free-form project memory. Materializes to the `## Project Memory`
    /// section of `CLAUDE.md` / `AGENTS.md`. Atomr long-term memory
    /// summaries land here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_memory: Option<String>,
}

impl ConceptProjection {
    pub fn is_empty(&self) -> bool {
        self.persona.is_none()
            && self.skills.is_empty()
            && self.toolsets.is_empty()
            && self.project_memory.is_none()
            && self.policy.allowed_tools.is_empty()
            && self.policy.allowed_models.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_projection_is_empty() {
        assert!(ConceptProjection::default().is_empty());
    }

    #[test]
    fn skill_round_trip() {
        let s = SkillSnapshot {
            id: "rag".into(),
            name: "RAG".into(),
            instruction_fragment: "use the index".into(),
            keywords: vec!["search".into()],
            priority: 7,
            tools: vec!["WebSearch".into()],
        };
        let j = serde_json::to_string(&s).unwrap();
        let back: SkillSnapshot = serde_json::from_str(&j).unwrap();
        assert_eq!(back.id, "rag");
        assert_eq!(back.priority, 7);
    }
}
