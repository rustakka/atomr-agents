//! Filesystem write helpers for the editing endpoints.
//!
//! Reads go through the host crate's existing readers; these cover the
//! write-back paths the host library doesn't expose as first-class methods
//! (per-doc resolution, SKILL.md writes, deletes).

use std::path::{Path, PathBuf};

use atomr_agents_host::error::{HostError, HostResult};
use atomr_agents_host::layout::AgentPaths;
use atomr_agents_host::markdown::MarkdownDoc;

/// Resolve one of the four canonical doc files by short name.
pub fn doc_path(paths: &AgentPaths, doc: &str) -> Option<PathBuf> {
    match doc {
        "soul" => Some(paths.soul_md()),
        "rules" => Some(paths.rules_md()),
        "memory" => Some(paths.memory_md()),
        "user" => Some(paths.user_md()),
        _ => None,
    }
}

/// Write a SOUL/RULES/MEMORY/USER doc back to disk.
pub fn write_doc(paths: &AgentPaths, doc: &str, content: &MarkdownDoc) -> HostResult<PathBuf> {
    let path = doc_path(paths, doc)
        .ok_or_else(|| HostError::config(format!("unknown doc `{doc}`")))?;
    content.write(&path)?;
    Ok(path)
}

/// Path to a skill's SKILL.md.
pub fn skill_md_path(paths: &AgentPaths, skill_id: &str) -> PathBuf {
    paths.skills_dir().join(skill_id).join("SKILL.md")
}

/// Write (create or overwrite) a skill's SKILL.md.
pub fn write_skill(paths: &AgentPaths, skill_id: &str, content: &MarkdownDoc) -> HostResult<PathBuf> {
    let path = skill_md_path(paths, skill_id);
    content.write(&path)?;
    Ok(path)
}

/// Remove a skill directory entirely.
pub fn delete_skill(paths: &AgentPaths, skill_id: &str) -> HostResult<bool> {
    let dir = paths.skills_dir().join(skill_id);
    if dir.is_dir() {
        std::fs::remove_dir_all(&dir).map_err(|e| HostError::io(&dir, e))?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Remove a single file (e.g. a cron or mcp yaml). Returns whether it existed.
pub fn remove_file(path: &Path) -> HostResult<bool> {
    if path.is_file() {
        std::fs::remove_file(path).map_err(|e| HostError::io(path, e))?;
        Ok(true)
    } else {
        Ok(false)
    }
}
