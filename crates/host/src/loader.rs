//! Read an agent's on-disk directory and assemble matching native
//! [`atomr_agents_agent::AgentSpec`], [`atomr_agents_skill::SkillSet`],
//! and [`atomr_agents_persona::Persona`] objects.

use std::collections::BTreeMap;
use std::path::PathBuf;

use semver::Version;
use serde::{Deserialize, Serialize};

use atomr_agents_agent::AgentSpec;
use atomr_agents_core::{AgentId, SkillId, ToolId};
use atomr_agents_persona::{Persona, PersonaMetadata, StyleSpec, TraitFragment};
use atomr_agents_skill::{Skill, SkillSet};

use crate::config::HostConfig;
use crate::error::{HostError, HostResult};
use crate::layout::AgentPaths;
use crate::markdown::{split_bullets, MarkdownDoc};

// ---------- pure-data definitions -------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDefinition {
    pub id: String,
    pub name: String,
    pub instruction_fragment: String,
    #[serde(default = "default_priority")]
    pub priority: u8,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub tool_overlay: Vec<String>,
    #[serde(default)]
    pub memory_namespace: Vec<String>,
    #[serde(default)]
    pub source_path: Option<PathBuf>,
}

fn default_priority() -> u8 {
    5
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookDefinition {
    pub event: String,
    #[serde(default)]
    pub match_: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub call: BTreeMap<String, serde_json::Value>,
    #[serde(default = "default_when")]
    pub when: String,
    #[serde(default)]
    pub budget: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub source_path: Option<PathBuf>,
}

fn default_when() -> String {
    "post".into()
}

#[derive(Debug, Clone)]
pub struct AgentDefinition {
    pub paths: AgentPaths,
    pub spec_yaml: BTreeMap<String, serde_json::Value>,
    pub soul: MarkdownDoc,
    pub rules: MarkdownDoc,
    pub memory: MarkdownDoc,
    pub user: MarkdownDoc,
    pub skills: Vec<SkillDefinition>,
    pub hooks: Vec<HookDefinition>,
}

impl AgentDefinition {
    pub fn agent_id(&self) -> String {
        self.spec_yaml
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.paths.agent_id.clone())
    }

    pub fn model(&self) -> Option<String> {
        self.spec_yaml
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    pub fn max_iterations(&self) -> u32 {
        self.spec_yaml
            .get("max_iterations")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32)
            .unwrap_or(8)
    }

    pub fn token_budget(&self) -> u32 {
        self.spec_yaml
            .get("token_budget")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32)
            .unwrap_or(8000)
    }

    pub fn time_budget_ms(&self) -> u64 {
        self.spec_yaml
            .get("time_budget_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(60_000)
    }

    pub fn money_budget_usd(&self) -> f64 {
        self.spec_yaml
            .get("money_budget_usd")
            .and_then(|v| v.as_f64())
            .unwrap_or(1.0)
    }

    pub fn skillset_id(&self) -> String {
        self.spec_yaml
            .get("skillset_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{}-skills", self.agent_id()))
    }

    pub fn skillset_version(&self) -> Version {
        let raw = self
            .spec_yaml
            .get("skillset_version")
            .and_then(|v| v.as_str())
            .unwrap_or("0.1.0");
        Version::parse(raw).unwrap_or_else(|_| Version::new(0, 1, 0))
    }
}

/// An [`AgentDefinition`] plus the native objects assembled from it.
#[derive(Clone)]
pub struct LoadedAgent {
    pub definition: AgentDefinition,
    pub spec: AgentSpec,
    pub skill_set: SkillSet,
    pub persona: Option<Persona>,
    pub rules: Vec<String>,
    pub memory_facts: Vec<String>,
    pub user_profile: String,
}

impl std::fmt::Debug for LoadedAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedAgent")
            .field("agent_id", &self.spec.id.as_str())
            .field("model", &self.spec.model)
            .field("skills", &self.skill_set.skills.len())
            .field("rules", &self.rules.len())
            .field("memory_facts", &self.memory_facts.len())
            .finish()
    }
}

/// Read agent directories under a [`HostConfig`].
#[derive(Debug, Clone)]
pub struct AgentLoader {
    config: HostConfig,
}

impl AgentLoader {
    pub fn new(config: HostConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &HostConfig {
        &self.config
    }

    pub fn agent_ids(&self) -> Vec<String> {
        self.config.paths.list_agent_ids()
    }

    /// Parse the agent directory into a pure-data [`AgentDefinition`].
    pub fn parse(&self, agent_id: &str) -> HostResult<AgentDefinition> {
        let paths = self.config.paths.agent(agent_id);
        if !paths.dir().is_dir() {
            return Err(HostError::AgentNotFound(agent_id.to_string(), paths.dir()));
        }

        let spec_yaml = read_agent_yaml(&paths.agent_yaml())?;
        let soul = MarkdownDoc::read(&paths.soul_md())?;
        let rules = MarkdownDoc::read(&paths.rules_md())?;
        let memory = MarkdownDoc::read(&paths.memory_md())?;
        let user = MarkdownDoc::read(&paths.user_md())?;

        let skills = read_skills(&paths.skills_dir())?;
        let hooks = read_hooks(&paths.hooks_dir())?;

        Ok(AgentDefinition {
            paths,
            spec_yaml,
            soul,
            rules,
            memory,
            user,
            skills,
            hooks,
        })
    }

    /// Parse + materialize native types.
    pub fn load(&self, agent_id: &str) -> HostResult<LoadedAgent> {
        let definition = self.parse(agent_id)?;
        let model = definition
            .model()
            .or_else(|| self.config.default_model.clone())
            .ok_or_else(|| {
                HostError::agent_spec(format!(
                    "agent {}: no `model` in agent.yaml and no `default_model` in config.yaml",
                    definition.agent_id()
                ))
            })?;

        let spec = AgentSpec {
            id: AgentId::from(definition.agent_id()),
            model,
            max_iterations: definition.max_iterations(),
            token_budget: definition.token_budget(),
            time_budget_ms: definition.time_budget_ms(),
            money_budget_usd: definition.money_budget_usd(),
        };

        let skill_set = build_skill_set(&definition);
        let persona = build_persona(&definition);
        let rules = split_bullets(&definition.rules.body);
        let memory_facts = split_bullets(&definition.memory.body);
        let user_profile = definition.user.body.clone();

        Ok(LoadedAgent {
            definition,
            spec,
            skill_set,
            persona,
            rules,
            memory_facts,
            user_profile,
        })
    }
}

// ---------- helpers ----------------------------------------------------------

fn read_agent_yaml(path: &std::path::Path) -> HostResult<BTreeMap<String, serde_json::Value>> {
    if !path.is_file() {
        return Err(HostError::agent_spec(format!("missing {}", path.display())));
    }
    let text = std::fs::read_to_string(path).map_err(|e| HostError::io(path, e))?;
    let raw: serde_yaml::Value = if text.trim().is_empty() {
        serde_yaml::Value::Mapping(Default::default())
    } else {
        serde_yaml::from_str(&text).map_err(|e| HostError::yaml(path, e))?
    };
    let map = match raw {
        serde_yaml::Value::Mapping(m) => m,
        serde_yaml::Value::Null => Default::default(),
        _ => {
            return Err(HostError::agent_spec(format!(
                "{}: top-level must be a YAML mapping",
                path.display()
            )));
        }
    };
    let mut out = BTreeMap::new();
    for (k, v) in map {
        if let Some(ks) = k.as_str() {
            out.insert(ks.to_string(), crate::config::yaml_to_json(v));
        }
    }
    Ok(out)
}

fn read_skills(skills_dir: &std::path::Path) -> HostResult<Vec<SkillDefinition>> {
    if !skills_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(skills_dir)
        .map_err(|e| HostError::io(skills_dir, e))?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let name = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue,
        };
        if name.starts_with('.') {
            continue;
        }
        if !path.is_dir() {
            continue;
        }
        let md_path = path.join("SKILL.md");
        if !md_path.is_file() {
            continue;
        }
        let doc = MarkdownDoc::read(&md_path)?;
        let fm = &doc.frontmatter;
        let display_name = fm
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| name.clone());
        let priority = fm
            .get("priority")
            .and_then(|v| v.as_u64())
            .map(|n| n.min(255) as u8)
            .unwrap_or(5);
        let keywords = coerce_str_list(fm.get("keywords"), "keywords", &md_path)?;
        let tool_overlay = coerce_str_list(fm.get("tool_overlay"), "tool_overlay", &md_path)?;
        let memory_namespace =
            coerce_str_list(fm.get("memory_namespace"), "memory_namespace", &md_path)?;
        out.push(SkillDefinition {
            id: name,
            name: display_name,
            instruction_fragment: doc.body,
            priority,
            keywords,
            tool_overlay,
            memory_namespace,
            source_path: Some(md_path),
        });
    }
    Ok(out)
}

fn coerce_str_list(
    value: Option<&serde_json::Value>,
    field: &str,
    path: &std::path::Path,
) -> HostResult<Vec<String>> {
    match value {
        None | Some(serde_json::Value::Null) => Ok(Vec::new()),
        Some(serde_json::Value::Array(arr)) => {
            let mut out = Vec::with_capacity(arr.len());
            for v in arr {
                match v {
                    serde_json::Value::String(s) => out.push(s.clone()),
                    _ => {
                        return Err(HostError::markdown(
                            path.to_path_buf(),
                            format!("`{field}` must be a list of strings"),
                        ));
                    }
                }
            }
            Ok(out)
        }
        Some(_) => Err(HostError::markdown(
            path.to_path_buf(),
            format!("`{field}` must be a list of strings"),
        )),
    }
}

fn read_hooks(hooks_dir: &std::path::Path) -> HostResult<Vec<HookDefinition>> {
    if !hooks_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(hooks_dir)
        .map_err(|e| HostError::io(hooks_dir, e))?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        if !matches!(ext, "yaml" | "yml") {
            continue;
        }
        let text = std::fs::read_to_string(&path).map_err(|e| HostError::io(&path, e))?;
        let raw: serde_yaml::Value = if text.trim().is_empty() {
            serde_yaml::Value::Mapping(Default::default())
        } else {
            serde_yaml::from_str(&text).map_err(|e| HostError::yaml(&path, e))?
        };
        let map = match raw {
            serde_yaml::Value::Mapping(m) => m,
            serde_yaml::Value::Null => Default::default(),
            _ => {
                return Err(HostError::hook(path, "top-level must be a YAML mapping"));
            }
        };
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("hook")
            .to_string();
        let event = map
            .get(serde_yaml::Value::String("event".into()))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or(stem);
        let when = map
            .get(serde_yaml::Value::String("when".into()))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(default_when);
        let match_ = yaml_mapping_to_json(map.get(serde_yaml::Value::String("match".into())))?;
        let call = yaml_mapping_to_json(map.get(serde_yaml::Value::String("call".into())))?;
        let budget = yaml_mapping_to_json(map.get(serde_yaml::Value::String("budget".into())))?;
        out.push(HookDefinition {
            event,
            match_,
            call,
            when,
            budget,
            source_path: Some(path),
        });
    }
    Ok(out)
}

fn yaml_mapping_to_json(
    v: Option<&serde_yaml::Value>,
) -> HostResult<BTreeMap<String, serde_json::Value>> {
    match v {
        None | Some(serde_yaml::Value::Null) => Ok(BTreeMap::new()),
        Some(serde_yaml::Value::Mapping(m)) => {
            let mut out = BTreeMap::new();
            for (k, v) in m {
                if let Some(ks) = k.as_str() {
                    out.insert(ks.to_string(), crate::config::yaml_to_json(v.clone()));
                }
            }
            Ok(out)
        }
        Some(_) => Err(HostError::AgentSpec(
            "match/call/budget must be mappings".into(),
        )),
    }
}

fn build_skill_set(def: &AgentDefinition) -> SkillSet {
    let skills: Vec<Skill> = def
        .skills
        .iter()
        .map(|sd| Skill {
            id: SkillId::from(sd.id.clone()),
            name: sd.name.clone(),
            instruction_fragment: sd.instruction_fragment.clone(),
            tool_overlay: sd.tool_overlay.iter().cloned().map(ToolId::from).collect(),
            memory_namespace: None,
            keywords: sd.keywords.clone(),
            priority: sd.priority,
        })
        .collect();
    SkillSet::new(def.skillset_id(), def.skillset_version(), skills)
}

fn build_persona(def: &AgentDefinition) -> Option<Persona> {
    if def.soul.is_empty() {
        return None;
    }
    let fm = &def.soul.frontmatter;
    let identity = fm
        .get("identity")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| def.agent_id());

    let style = match fm.get("style") {
        Some(serde_json::Value::Object(m)) => StyleSpec {
            tone: m.get("tone").and_then(|v| v.as_str()).map(|s| s.to_string()),
            register: m
                .get("register")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            verbosity: m
                .get("verbosity")
                .and_then(|v| v.as_u64())
                .map(|n| n.min(255) as u8),
        },
        _ => StyleSpec::default(),
    };

    let salient_traits = match fm.get("traits") {
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| {
                let m = v.as_object()?;
                Some(TraitFragment {
                    label: m
                        .get("label")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    weight: m.get("weight").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32,
                    description: m
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                })
            })
            .collect(),
        _ => Vec::new(),
    };

    let metadata = match fm.get("metadata") {
        Some(serde_json::Value::Object(m)) => PersonaMetadata {
            framework: m
                .get("framework")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        },
        _ => PersonaMetadata::default(),
    };

    Some(Persona {
        identity,
        salient_traits,
        style,
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write(p: &std::path::Path, body: &str) {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, body).unwrap();
    }

    fn fixture_agent(root: &std::path::Path) {
        let agent = root.join("agents").join("alpha");
        write(
            &agent.join("agent.yaml"),
            "id: alpha\nmodel: gpt-4o\nmax_iterations: 4\ntoken_budget: 1234\n",
        );
        write(
            &agent.join("SOUL.md"),
            "---\nidentity: Alpha\nstyle:\n  tone: terse\n---\nA terse agent.\n",
        );
        write(
            &agent.join("RULES.md"),
            "# Rules\n\n- be helpful\n- avoid secrets\n",
        );
        write(&agent.join("MEMORY.md"), "- fact one\n- fact two\n");
        write(&agent.join("USER.md"), "user is Matt\n");
        write(
            &agent.join("skills/summarize/SKILL.md"),
            "---\nname: Summarize\npriority: 7\nkeywords:\n  - tldr\n  - summarize\n---\nProduce concise summaries.\n",
        );
    }

    #[test]
    fn parse_returns_agent_definition() {
        let tmp = tempdir().unwrap();
        fixture_agent(tmp.path());
        let cfg = HostConfig::load(tmp.path()).unwrap();
        let loader = AgentLoader::new(cfg);
        let def = loader.parse("alpha").unwrap();
        assert_eq!(def.agent_id(), "alpha");
        assert_eq!(def.model().as_deref(), Some("gpt-4o"));
        assert_eq!(def.max_iterations(), 4);
        assert_eq!(def.token_budget(), 1234);
        assert_eq!(def.skills.len(), 1);
        assert_eq!(def.skills[0].keywords, vec!["tldr", "summarize"]);
    }

    #[test]
    fn load_builds_native_objects() {
        let tmp = tempdir().unwrap();
        fixture_agent(tmp.path());
        let cfg = HostConfig::load(tmp.path()).unwrap();
        let loader = AgentLoader::new(cfg);
        let loaded = loader.load("alpha").unwrap();
        assert_eq!(loaded.spec.id.as_str(), "alpha");
        assert_eq!(loaded.skill_set.skills.len(), 1);
        assert!(loaded.persona.is_some());
        assert_eq!(loaded.rules, vec!["be helpful", "avoid secrets"]);
        assert_eq!(loaded.memory_facts, vec!["fact one", "fact two"]);
    }

    #[test]
    fn missing_agent_errors() {
        let tmp = tempdir().unwrap();
        let cfg = HostConfig::load(tmp.path()).unwrap();
        let loader = AgentLoader::new(cfg);
        match loader.parse("ghost") {
            Err(HostError::AgentNotFound(id, _)) => assert_eq!(id, "ghost"),
            other => panic!("expected AgentNotFound, got {other:?}"),
        }
    }
}
