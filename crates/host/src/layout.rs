//! On-disk layout for an atomr-agents host root.
//!
//! Default root: `~/.atomr/host/` (overridable via `$ATOMR_HOST_ROOT`).

use std::env;
use std::path::{Path, PathBuf};

use crate::error::{HostError, HostResult};

pub const ENV_ROOT: &str = "ATOMR_HOST_ROOT";

/// Resolve the default host root, honoring `$ATOMR_HOST_ROOT`.
///
/// Does **not** create the directory; callers (`init` and
/// [`HostPaths::ensure`]) own creation so cold lookups stay read-only.
pub fn default_root() -> PathBuf {
    if let Ok(env) = env::var(ENV_ROOT) {
        let p = PathBuf::from(env);
        return expand_user(&p);
    }
    let home = dirs_home();
    home.join(".atomr").join("host")
}

fn dirs_home() -> PathBuf {
    if let Ok(home) = env::var("HOME") {
        return PathBuf::from(home);
    }
    // Final fallback — useful in CI / sandboxes that strip HOME.
    PathBuf::from("/tmp")
}

fn expand_user(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/") {
        return dirs_home().join(rest);
    }
    if s == "~" {
        return dirs_home();
    }
    p.to_path_buf()
}

/// Filesystem layout for the host root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPaths {
    pub root: PathBuf,
}

impl HostPaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = expand_user(&root.into());
        Self { root }
    }

    pub fn config_yaml(&self) -> PathBuf {
        self.root.join("config.yaml")
    }

    pub fn agents_md(&self) -> PathBuf {
        self.root.join("AGENTS.md")
    }

    pub fn agents_dir(&self) -> PathBuf {
        self.root.join("agents")
    }

    pub fn channels_dir(&self) -> PathBuf {
        self.root.join("channels")
    }

    pub fn crons_dir(&self) -> PathBuf {
        self.root.join("crons")
    }

    pub fn tools_dir(&self) -> PathBuf {
        self.root.join("tools")
    }

    pub fn registry_dir(&self) -> PathBuf {
        self.root.join("registry")
    }

    pub fn evals_dir(&self) -> PathBuf {
        self.root.join("evals")
    }

    pub fn events_jsonl(&self) -> PathBuf {
        self.root.join("events.jsonl")
    }

    pub fn mcp_dir(&self) -> PathBuf {
        self.root.join("mcp")
    }

    pub fn agent(&self, agent_id: &str) -> AgentPaths {
        AgentPaths {
            root: self.root.clone(),
            agent_id: agent_id.to_string(),
        }
    }

    /// Enumerate agent ids by scanning `agents/` for sub-directories.
    /// Returns an empty list when the host root does not exist yet.
    pub fn list_agent_ids(&self) -> Vec<String> {
        let agents = self.agents_dir();
        if !agents.is_dir() {
            return Vec::new();
        }
        let mut out: Vec<String> = match std::fs::read_dir(&agents) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|name| !name.starts_with('.'))
                .collect(),
            Err(_) => Vec::new(),
        };
        out.sort();
        out
    }

    /// Create the host directory skeleton (idempotent).
    pub fn ensure(&self) -> HostResult<()> {
        for d in [
            self.root.clone(),
            self.agents_dir(),
            self.channels_dir(),
            self.crons_dir(),
            self.tools_dir(),
            self.registry_dir(),
        ] {
            std::fs::create_dir_all(&d).map_err(|e| HostError::io(&d, e))?;
        }
        Ok(())
    }
}

/// Filesystem layout for a single agent under `<root>/agents/<id>/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPaths {
    pub root: PathBuf,
    pub agent_id: String,
}

impl AgentPaths {
    pub fn dir(&self) -> PathBuf {
        self.root.join("agents").join(&self.agent_id)
    }
    pub fn agent_yaml(&self) -> PathBuf {
        self.dir().join("agent.yaml")
    }
    pub fn soul_md(&self) -> PathBuf {
        self.dir().join("SOUL.md")
    }
    pub fn rules_md(&self) -> PathBuf {
        self.dir().join("RULES.md")
    }
    pub fn memory_md(&self) -> PathBuf {
        self.dir().join("MEMORY.md")
    }
    pub fn user_md(&self) -> PathBuf {
        self.dir().join("USER.md")
    }
    pub fn skills_dir(&self) -> PathBuf {
        self.dir().join("skills")
    }
    pub fn hooks_dir(&self) -> PathBuf {
        self.dir().join("hooks")
    }
    pub fn state_dir(&self) -> PathBuf {
        self.dir().join("state")
    }
    pub fn threads_dir(&self) -> PathBuf {
        self.state_dir().join("threads")
    }
    pub fn checkpoints_dir(&self) -> PathBuf {
        self.state_dir().join("checkpoints")
    }
    pub fn memory_db(&self) -> PathBuf {
        self.state_dir().join("memory.db")
    }

    pub fn ensure(&self) -> HostResult<()> {
        for d in [
            self.dir(),
            self.skills_dir(),
            self.hooks_dir(),
            self.state_dir(),
            self.threads_dir(),
            self.checkpoints_dir(),
        ] {
            std::fs::create_dir_all(&d).map_err(|e| HostError::io(&d, e))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn host_paths_basics() {
        let tmp = tempdir().unwrap();
        let paths = HostPaths::new(tmp.path());
        assert_eq!(paths.config_yaml(), tmp.path().join("config.yaml"));
        assert_eq!(paths.agents_dir(), tmp.path().join("agents"));
        assert_eq!(paths.events_jsonl(), tmp.path().join("events.jsonl"));
    }

    #[test]
    fn agent_paths_under_root() {
        let tmp = tempdir().unwrap();
        let paths = HostPaths::new(tmp.path()).agent("alpha");
        assert_eq!(paths.agent_yaml(), tmp.path().join("agents/alpha/agent.yaml"));
        assert_eq!(paths.soul_md(), tmp.path().join("agents/alpha/SOUL.md"));
        assert_eq!(
            paths.checkpoints_dir(),
            tmp.path().join("agents/alpha/state/checkpoints")
        );
    }

    #[test]
    fn ensure_idempotent() {
        let tmp = tempdir().unwrap();
        let paths = HostPaths::new(tmp.path());
        paths.ensure().unwrap();
        paths.ensure().unwrap();
        assert!(paths.agents_dir().is_dir());
    }

    #[test]
    fn list_agent_ids_filters_hidden() {
        let tmp = tempdir().unwrap();
        let paths = HostPaths::new(tmp.path());
        paths.ensure().unwrap();
        std::fs::create_dir_all(paths.agents_dir().join("alpha")).unwrap();
        std::fs::create_dir_all(paths.agents_dir().join(".hidden")).unwrap();
        std::fs::create_dir_all(paths.agents_dir().join("beta")).unwrap();
        assert_eq!(paths.list_agent_ids(), vec!["alpha", "beta"]);
    }
}
