//! Serializable DTOs and the unified concept catalog.
//!
//! The host's runtime snapshot types ([`IdentitySnapshot`] etc.) aren't
//! `Serialize`, and [`LoadedAgent`] is a heavy composite, so this module
//! defines the flat shapes the web API actually returns. Where an existing
//! host type already derives `Serialize` (skills, hooks, crons, checkpoints,
//! eval suites, MCP configs, event records) we re-use it directly.

use atomr_agents_host::loader::{HookDefinition, LoadedAgent, SkillDefinition};
use atomr_agents_host::markdown::MarkdownDoc;
use serde::{Deserialize, Serialize};

/// One-line agent card / row.
#[derive(Debug, Clone, Serialize)]
pub struct AgentSummary {
    pub id: String,
    pub model: String,
    pub persona_identity: Option<String>,
    pub running: bool,
    pub rules_count: usize,
    pub memory_facts_count: usize,
    pub skills_count: usize,
    pub user_profile_len: usize,
}

impl AgentSummary {
    pub fn from_loaded(loaded: &LoadedAgent, running: bool) -> Self {
        Self {
            id: loaded.spec.id.to_string(),
            model: loaded.spec.model.clone(),
            persona_identity: loaded.persona.as_ref().map(|p| p.identity.clone()),
            running,
            rules_count: loaded.rules.len(),
            memory_facts_count: loaded.memory_facts.len(),
            skills_count: loaded.skill_set.skills.len(),
            user_profile_len: loaded.user_profile.len(),
        }
    }
}

/// The four human-readable Markdown docs that define an agent.
#[derive(Debug, Clone, Serialize)]
pub struct AgentDocs {
    pub soul: MarkdownDoc,
    pub rules: MarkdownDoc,
    pub memory: MarkdownDoc,
    pub user: MarkdownDoc,
}

/// Full agent view backing the detail page.
#[derive(Debug, Clone, Serialize)]
pub struct AgentDetail {
    pub id: String,
    pub model: String,
    pub running: bool,
    pub spec: serde_json::Value,
    pub summary: AgentSummary,
    pub docs: AgentDocs,
    pub skills: Vec<SkillDefinition>,
    pub hooks: Vec<HookDefinition>,
}

impl AgentDetail {
    pub fn from_loaded(loaded: &LoadedAgent, running: bool) -> Self {
        let def = &loaded.definition;
        Self {
            id: loaded.spec.id.to_string(),
            model: loaded.spec.model.clone(),
            running,
            spec: serde_json::to_value(&def.spec_yaml).unwrap_or(serde_json::Value::Null),
            summary: AgentSummary::from_loaded(loaded, running),
            docs: AgentDocs {
                soul: def.soul.clone(),
                rules: def.rules.clone(),
                memory: def.memory.clone(),
                user: def.user.clone(),
            },
            skills: def.skills.clone(),
            hooks: def.hooks.clone(),
        }
    }
}

/// Body for `PUT /api/agents/:id/docs/:doc` and skill SKILL.md edits.
#[derive(Debug, Clone, Deserialize)]
pub struct DocUpdate {
    #[serde(default)]
    pub frontmatter: std::collections::BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub body: String,
}

impl DocUpdate {
    pub fn into_doc(self) -> MarkdownDoc {
        MarkdownDoc {
            source_path: None,
            frontmatter: self.frontmatter,
            body: self.body,
        }
    }
}

/// One entry in the unified concept catalog — merges Hermes / OpenClaw /
/// AionUi vocabulary onto an existing host primitive.
#[derive(Debug, Clone, Serialize)]
pub struct Concept {
    pub key: &'static str,
    pub label: &'static str,
    pub primitive: &'static str,
    pub borrowed_from: &'static str,
    pub api: &'static str,
    pub ui_section: &'static str,
    pub description: &'static str,
}

/// The canonical concept catalog served at `GET /api/concepts` and rendered
/// on the Concepts page. This is the single source of truth the sidebar and
/// `docs/agent-host/concepts.md` are organized around.
pub fn concept_catalog() -> Vec<Concept> {
    vec![
        Concept {
            key: "agent",
            label: "Agent",
            primitive: "AgentSpec + AgentHostActor",
            borrowed_from: "AionUi Assistants/Agents",
            api: "/api/agents",
            ui_section: "Agents",
            description: "A long-lived identity with model, budgets, persona, rules, memory, skills and hooks.",
        },
        Concept {
            key: "identity",
            label: "Identity (SOUL)",
            primitive: "SOUL.md → Persona",
            borrowed_from: "OpenClaw / Hermes SOUL",
            api: "/api/agents/:id/docs/soul",
            ui_section: "Agent ▸ Identity",
            description: "Human-readable Markdown defining who the agent is; frontmatter materializes a Persona.",
        },
        Concept {
            key: "rules",
            label: "Rules",
            primitive: "RULES.md → instruction prefix",
            borrowed_from: "OpenClaw / Hermes RULES",
            api: "/api/agents/:id/docs/rules",
            ui_section: "Agent ▸ Rules",
            description: "Bulleted behavioral constraints rendered into the system prompt.",
        },
        Concept {
            key: "memory",
            label: "Memory",
            primitive: "MEMORY.md → memory facts",
            borrowed_from: "OpenClaw / Hermes MEMORY",
            api: "/api/agents/:id/docs/memory",
            ui_section: "Agent ▸ Memory",
            description: "Durable facts the agent recalls across turns and sessions.",
        },
        Concept {
            key: "user",
            label: "User profile",
            primitive: "USER.md → user profile",
            borrowed_from: "OpenClaw / Hermes USER",
            api: "/api/agents/:id/docs/user",
            ui_section: "Agent ▸ User",
            description: "What the agent knows about the human it serves.",
        },
        Concept {
            key: "skill",
            label: "Skill",
            primitive: "SKILL.md → Skill / SkillSet",
            borrowed_from: "AionUi Skills / Hermes auto-curated skills",
            api: "/api/agents/:id/skills",
            ui_section: "Agent ▸ Skills",
            description: "A bundled instruction fragment + tool overlay + keywords, selected by relevance.",
        },
        Concept {
            key: "curator",
            label: "Curator",
            primitive: "SkillProposal + CurationStrategy",
            borrowed_from: "Hermes auto-curation",
            api: "/api/agents/:id/curator/proposals",
            ui_section: "Agent ▸ Skills",
            description: "Proposed skills awaiting promotion, with versioned history and revert.",
        },
        Concept {
            key: "hook",
            label: "Hook",
            primitive: "HookDefinition + HookDispatcher",
            borrowed_from: "Claude Code hooks",
            api: "/api/agents/:id/hooks",
            ui_section: "Agent ▸ Hooks",
            description: "Event-matched actions dispatched pre/post an agent event.",
        },
        Concept {
            key: "cron",
            label: "Cron",
            primitive: "CronEntry + Scheduler",
            borrowed_from: "AionUi Scheduled Tasks / Hermes heartbeat",
            api: "/api/crons",
            ui_section: "Crons",
            description: "Scheduled fires (e.g. every:5m) that wake an agent with an input.",
        },
        Concept {
            key: "route",
            label: "Channels & Routing",
            primitive: "AGENTS.md → AgentRouter / Gateway",
            borrowed_from: "OpenClaw gateway / Hermes 20+ channels",
            api: "/api/routes",
            ui_section: "Channels & Routing",
            description: "Which agent answers on which channel/peer — one agent across CLI/WhatsApp/Discord.",
        },
        Concept {
            key: "branch",
            label: "Branch / Checkpoint",
            primitive: "Checkpoint + branch ops",
            borrowed_from: "Claude Code checkpoints",
            api: "/api/agents/:id/branches",
            ui_section: "Agent ▸ Branches",
            description: "Snapshots of working memory you can fork, switch, diff and revert.",
        },
        Concept {
            key: "registry",
            label: "Registry artifact",
            primitive: "CachedArtifact",
            borrowed_from: "atomr registry",
            api: "/api/registry",
            ui_section: "Registry",
            description: "Versioned, cached tool sets / skills / personas / agents pulled from a registry.",
        },
        Concept {
            key: "eval",
            label: "Eval suite",
            primitive: "EvalSuite + run_suite",
            borrowed_from: "atomr eval",
            api: "/api/evals",
            ui_section: "Agent ▸ Evals",
            description: "Cases scored against an agent's responses to catch regressions.",
        },
        Concept {
            key: "mcp",
            label: "MCP server",
            primitive: "MCPServerConfig + McpBridge",
            borrowed_from: "AionUi MCP integration",
            api: "/api/mcp",
            ui_section: "MCP",
            description: "Model Context Protocol tool servers the agent can call.",
        },
        Concept {
            key: "event",
            label: "Event",
            primitive: "EventRecord + EventLog",
            borrowed_from: "Claude Code event log",
            api: "/api/events",
            ui_section: "Events",
            description: "Append-only JSONL stream of everything that happens in the host.",
        },
        Concept {
            key: "config",
            label: "Config",
            primitive: "HostConfig (config.yaml)",
            borrowed_from: "—",
            api: "/api/config",
            ui_section: "Settings",
            description: "Host-wide defaults and inference provider entries.",
        },
    ]
}
