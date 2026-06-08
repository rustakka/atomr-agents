//! Materialize atomr concepts into a `.claude/` directory the SDK reads.
//!
//! With `setting_sources = ["project"]`, the Claude Agent SDK loads
//! `.claude/skills/`, `.claude/commands/`, `.claude/settings.local.json`,
//! and `.mcp.json` from the working directory. This module writes those
//! deterministically before a run so the agent sees atomr-controlled
//! config. It mirrors the projection done by
//! `coding-cli-vendor-claude::mapper` but is self-contained (no
//! coding-cli dependency).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use atomr_agents_agent_sdk_core::McpServerConfig;

/// One skill rendered to `.claude/skills/<id>/SKILL.md`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillDoc {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub body: String,
}

/// One slash command rendered to `.claude/commands/<name>.md`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommandDoc {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub body: String,
}

/// Everything materialized into `cwd/.claude/` (+ `.mcp.json`) before a run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Projection {
    #[serde(default)]
    pub skills: Vec<SkillDoc>,
    #[serde(default)]
    pub commands: Vec<CommandDoc>,
    /// External MCP servers (in-process markers are skipped — those are
    /// injected by the Python wrapper, not written to `.mcp.json`).
    #[serde(default)]
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
    /// Written verbatim to `.claude/settings.local.json` when present.
    #[serde(default)]
    pub settings: Option<serde_json::Value>,
}

impl Projection {
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
            && self.commands.is_empty()
            && self.mcp_servers.is_empty()
            && self.settings.is_none()
    }
}

fn frontmatter_list(key: &str, items: &[String], out: &mut String) {
    if items.is_empty() {
        return;
    }
    out.push_str(key);
    out.push_str(":\n");
    for it in items {
        out.push_str("  - ");
        out.push_str(it);
        out.push('\n');
    }
}

/// Render the projection to a list of `(relative-path, bytes)` files. Pure —
/// no I/O. The relative paths use forward slashes (`Path::join` accepts them
/// on Windows too, and the sandbox `write_file` contract clamps them to root).
/// Empty when the projection is empty. This is the single source of truth for
/// *what* gets written; the host-fs and sandbox sinks below only decide *where*.
pub fn render_projection(p: &Projection) -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();

    // Skills → .claude/skills/<id>/SKILL.md
    for s in &p.skills {
        let mut md = String::from("---\n");
        md.push_str(&format!("name: {}\n", s.name.clone().unwrap_or_else(|| s.id.clone())));
        if let Some(d) = &s.description {
            md.push_str(&format!("description: {d}\n"));
        }
        frontmatter_list("allowed-tools", &s.allowed_tools, &mut md);
        md.push_str("---\n\n");
        md.push_str(&s.body);
        if !s.body.ends_with('\n') {
            md.push('\n');
        }
        files.push((format!(".claude/skills/{}/SKILL.md", s.id), md.into_bytes()));
    }

    // Slash commands → .claude/commands/<name>.md
    for c in &p.commands {
        let mut md = String::new();
        let has_fm = c.description.is_some() || !c.allowed_tools.is_empty() || c.model.is_some();
        if has_fm {
            md.push_str("---\n");
            if let Some(d) = &c.description {
                md.push_str(&format!("description: {d}\n"));
            }
            if let Some(m) = &c.model {
                md.push_str(&format!("model: {m}\n"));
            }
            frontmatter_list("allowed-tools", &c.allowed_tools, &mut md);
            md.push_str("---\n\n");
        }
        md.push_str(&c.body);
        if !c.body.ends_with('\n') {
            md.push('\n');
        }
        files.push((format!(".claude/commands/{}.md", c.name), md.into_bytes()));
    }

    // External MCP servers → .mcp.json (in-process markers skipped).
    let external: serde_json::Map<String, serde_json::Value> = p
        .mcp_servers
        .iter()
        .filter_map(|(name, cfg)| mcp_to_json(cfg).map(|v| (name.clone(), v)))
        .collect();
    if !external.is_empty() {
        let doc = serde_json::json!({ "mcpServers": external });
        files.push((".mcp.json".to_string(), serde_json::to_vec_pretty(&doc).unwrap_or_default()));
    }

    // Settings → .claude/settings.local.json
    if let Some(settings) = &p.settings {
        files.push((
            ".claude/settings.local.json".to_string(),
            serde_json::to_vec_pretty(settings).unwrap_or_default(),
        ));
    }

    files
}

/// Write the projection into `cwd` (the host filesystem). Idempotent —
/// overwrites existing files. An empty projection writes nothing.
pub fn materialize(cwd: &Path, p: &Projection) -> std::io::Result<()> {
    for (rel, bytes) in render_projection(p) {
        let full = cwd.join(&rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(full, bytes)?;
    }
    Ok(())
}

/// Stage the projection into a sandbox filesystem (Pattern C) via the
/// `SandboxHandle::write_file` contract — the same files as [`materialize`],
/// only the sink differs.
#[cfg(feature = "sandbox")]
pub async fn stage_into_sandbox(
    handle: &dyn atomr_agents_sandbox_core::SandboxHandle,
    p: &Projection,
) -> std::result::Result<(), atomr_agents_sandbox_core::SandboxError> {
    for (rel, bytes) in render_projection(p) {
        handle.write_file(&rel, &bytes).await?;
    }
    Ok(())
}

/// `.mcp.json` server shape. Returns `None` for in-process markers.
fn mcp_to_json(cfg: &McpServerConfig) -> Option<serde_json::Value> {
    match cfg {
        McpServerConfig::Stdio { command, args, env } => Some(serde_json::json!({
            "command": command,
            "args": args,
            "env": env,
        })),
        McpServerConfig::Sse { url, headers } => Some(serde_json::json!({
            "type": "sse",
            "url": url,
            "headers": headers,
        })),
        McpServerConfig::Http { url, headers } => Some(serde_json::json!({
            "type": "http",
            "url": url,
            "headers": headers,
        })),
        McpServerConfig::InProcess { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materialize_writes_claude_tree() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path();
        let mut mcp = BTreeMap::new();
        mcp.insert(
            "github".to_string(),
            McpServerConfig::Stdio {
                command: "npx".into(),
                args: vec!["-y".into(), "server".into()],
                env: Default::default(),
            },
        );
        mcp.insert("inproc".to_string(), McpServerConfig::InProcess { name: "atomr".into() });
        let p = Projection {
            skills: vec![SkillDoc {
                id: "review".into(),
                name: Some("Review".into()),
                description: Some("Reviews code".into()),
                allowed_tools: vec!["Read".into()],
                body: "Do a review.".into(),
            }],
            commands: vec![CommandDoc {
                name: "echo".into(),
                description: Some("Echo".into()),
                allowed_tools: vec![],
                model: None,
                body: "Echo: $ARGUMENTS".into(),
            }],
            mcp_servers: mcp,
            settings: Some(serde_json::json!({"permissions": {"allow": ["Bash"]}})),
        };
        materialize(cwd, &p).unwrap();

        assert!(cwd.join(".claude/skills/review/SKILL.md").is_file());
        assert!(cwd.join(".claude/commands/echo.md").is_file());
        assert!(cwd.join(".claude/settings.local.json").is_file());
        let mcp_doc = std::fs::read_to_string(cwd.join(".mcp.json")).unwrap();
        assert!(mcp_doc.contains("github"));
        // In-process markers are not written to .mcp.json.
        assert!(!mcp_doc.contains("inproc"));

        let skill = std::fs::read_to_string(cwd.join(".claude/skills/review/SKILL.md")).unwrap();
        assert!(skill.contains("name: Review"));
        assert!(skill.contains("allowed-tools:"));
    }

    #[test]
    fn empty_projection_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        materialize(dir.path(), &Projection::default()).unwrap();
        assert!(!dir.path().join(".claude").exists());
    }

    #[test]
    fn render_projection_emits_relative_forward_slash_paths() {
        let p = Projection {
            skills: vec![SkillDoc { id: "s".into(), ..Default::default() }],
            commands: vec![CommandDoc { name: "c".into(), ..Default::default() }],
            ..Default::default()
        };
        let files = render_projection(&p);
        let paths: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
        assert!(paths.contains(&".claude/skills/s/SKILL.md"));
        assert!(paths.contains(&".claude/commands/c.md"));
        // Empty projection renders no files.
        assert!(render_projection(&Projection::default()).is_empty());
    }
}
