//! Load a Claude Agent SDK harness spec + `.claude` projection from disk.
//!
//! Layout under `<root>/agent-sdk/<id>/`:
//!
//! ```text
//! harness.yaml          → AgentSdkHarnessSpec (optional; defaults if absent)
//! commands/<name>.md    → slash commands (frontmatter: description/model/allowed-tools)
//! skills/<id>/SKILL.md  → skills (frontmatter: name/description/allowed-tools)
//! mcp/<name>.yaml       → external MCP servers (McpServerConfig)
//! ```
//!
//! The result feeds
//! `AgentSdkHarness::new(backend, spec).with_projection(projection)` — the
//! projection materializes the `.claude/` tree into the run's `cwd` so the
//! SDK (with `setting_sources = ["project"]`) sees atomr-controlled config.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use atomr_agents_agent_sdk_harness::core::McpServerConfig;
use atomr_agents_agent_sdk_harness::{AgentSdkHarnessSpec, CommandDoc, Projection, SkillDoc};

use crate::error::{HostError, HostResult};

/// A harness spec + its `.claude` projection.
#[derive(Debug, Clone)]
pub struct LoadedAgentSdk {
    pub id: String,
    pub spec: AgentSdkHarnessSpec,
    pub projection: Projection,
}

#[derive(Debug, Default, Deserialize)]
struct Frontmatter {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default, rename = "allowed-tools")]
    allowed_tools: Vec<String>,
}

/// Split a `---\n<yaml>\n---\n<body>` markdown doc. Missing/invalid
/// frontmatter yields defaults + the whole text as the body.
fn split_frontmatter(text: &str) -> (Frontmatter, String) {
    let t = text.trim_start_matches('\u{feff}');
    if let Some(after) = t.strip_prefix("---") {
        if let Some(idx) = after.find("\n---") {
            let fm_str = after[..idx].trim_start_matches('\n');
            let after_fence = &after[idx + 4..]; // skip "\n---"
            let body = match after_fence.find('\n') {
                Some(nl) => &after_fence[nl + 1..],
                None => "",
            };
            let fm: Frontmatter = serde_yaml::from_str(fm_str).unwrap_or_default();
            return (fm, body.to_string());
        }
    }
    (Frontmatter::default(), text.to_string())
}

/// Load the spec + projection for one agent-sdk harness directory.
pub fn load_agent_sdk(id: &str, dir: &Path) -> HostResult<LoadedAgentSdk> {
    let spec = load_spec(dir)?;
    let projection = Projection {
        commands: load_commands(&dir.join("commands"))?,
        skills: load_skills(&dir.join("skills"))?,
        mcp_servers: load_mcp(&dir.join("mcp"))?,
        settings: None,
    };
    Ok(LoadedAgentSdk {
        id: id.to_string(),
        spec,
        projection,
    })
}

fn load_spec(dir: &Path) -> HostResult<AgentSdkHarnessSpec> {
    let p = dir.join("harness.yaml");
    if !p.is_file() {
        return Ok(AgentSdkHarnessSpec::default());
    }
    let s = std::fs::read_to_string(&p).map_err(|e| HostError::io(&p, e))?;
    serde_yaml::from_str(&s).map_err(|e| HostError::yaml(&p, e))
}

fn load_commands(dir: &Path) -> HostResult<Vec<CommandDoc>> {
    let mut cmds = Vec::new();
    for (name, content) in read_md_files(dir)? {
        let (fm, body) = split_frontmatter(&content);
        cmds.push(CommandDoc {
            name,
            description: fm.description,
            allowed_tools: fm.allowed_tools,
            model: fm.model,
            body,
        });
    }
    Ok(cmds)
}

fn load_skills(dir: &Path) -> HostResult<Vec<SkillDoc>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| HostError::io(dir, e))?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    let mut skills = Vec::new();
    for entry in entries {
        let sdir = entry.path();
        if !sdir.is_dir() {
            continue;
        }
        let skill_md = sdir.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        let id = sdir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let content = std::fs::read_to_string(&skill_md).map_err(|e| HostError::io(&skill_md, e))?;
        let (fm, body) = split_frontmatter(&content);
        skills.push(SkillDoc {
            id,
            name: fm.name,
            description: fm.description,
            allowed_tools: fm.allowed_tools,
            body,
        });
    }
    Ok(skills)
}

fn load_mcp(dir: &Path) -> HostResult<BTreeMap<String, McpServerConfig>> {
    if !dir.is_dir() {
        return Ok(BTreeMap::new());
    }
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| HostError::io(dir, e))?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    let mut map = BTreeMap::new();
    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let s = std::fs::read_to_string(&path).map_err(|e| HostError::io(&path, e))?;
        let cfg: McpServerConfig = serde_yaml::from_str(&s).map_err(|e| HostError::yaml(&path, e))?;
        map.insert(name, cfg);
    }
    Ok(map)
}

/// Read every `*.md` under `dir` as `(file_stem, contents)`, sorted by stem.
fn read_md_files(dir: &Path) -> HostResult<Vec<(String, String)>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| HostError::io(dir, e))? {
        let entry = entry.map_err(|e| HostError::io(dir, e))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let content = std::fs::read_to_string(&path).map_err(|e| HostError::io(&path, e))?;
            out.push((stem, content));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn loads_spec_commands_skills_mcp() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path().join("reviewer");
        std::fs::create_dir_all(dir.join("commands")).unwrap();
        std::fs::create_dir_all(dir.join("skills/review")).unwrap();
        std::fs::create_dir_all(dir.join("mcp")).unwrap();

        std::fs::write(
            dir.join("harness.yaml"),
            "id: reviewer\ndefault_model: claude-sonnet-4-6\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("commands/echo.md"),
            "---\ndescription: Echo input\nallowed-tools:\n  - Read\n---\nEcho: $ARGUMENTS\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("skills/review/SKILL.md"),
            "---\nname: Review\ndescription: Reviews code\n---\nReview carefully.\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("mcp/github.yaml"),
            "transport: stdio\ncommand: npx\nargs:\n  - -y\n  - server-github\n",
        )
        .unwrap();

        let loaded = load_agent_sdk("reviewer", &dir).unwrap();
        assert_eq!(loaded.spec.id, "reviewer");
        assert_eq!(loaded.spec.default_model.as_deref(), Some("claude-sonnet-4-6"));
        assert_eq!(loaded.projection.commands.len(), 1);
        assert_eq!(loaded.projection.commands[0].name, "echo");
        assert_eq!(loaded.projection.commands[0].allowed_tools, vec!["Read"]);
        assert_eq!(loaded.projection.skills.len(), 1);
        assert_eq!(loaded.projection.skills[0].id, "review");
        assert_eq!(loaded.projection.skills[0].name.as_deref(), Some("Review"));
        assert!(loaded.projection.skills[0].body.contains("Review carefully"));
        assert!(loaded.projection.mcp_servers.contains_key("github"));
    }

    #[test]
    fn missing_dir_yields_defaults() {
        let tmp = tempdir().unwrap();
        let loaded = load_agent_sdk("none", &tmp.path().join("none")).unwrap();
        assert_eq!(loaded.spec.id, "agent-sdk"); // default spec id
        assert!(loaded.projection.is_empty());
    }
}
