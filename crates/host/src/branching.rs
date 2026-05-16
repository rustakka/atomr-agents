//! M10 — Branching / checkpoints. JSON checkpoint files; switched-to
//! branch persisted in `state/checkpoints/CURRENT`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{HostError, HostResult};
use crate::layout::AgentPaths;

pub const DEFAULT_BRANCH: &str = "main";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub branch_id: String,
    pub agent_id: String,
    pub ts_ms: u64,
    pub working_memory: serde_json::Value,
    #[serde(default)]
    pub thread_head: Option<serde_json::Value>,
    #[serde(default)]
    pub parent_branch: Option<String>,
    #[serde(default)]
    pub path: Option<PathBuf>,
}

pub fn write_checkpoint(
    paths: &AgentPaths,
    branch_id: &str,
    working_memory: serde_json::Value,
    thread_head: Option<serde_json::Value>,
    parent_branch: Option<String>,
) -> HostResult<Checkpoint> {
    let dir = paths.checkpoints_dir().join(branch_id);
    std::fs::create_dir_all(&dir).map_err(|e| HostError::io(&dir, e))?;
    let ts = now_ms();
    let path = dir.join(format!("{ts}.json"));
    let cp = Checkpoint {
        branch_id: branch_id.to_string(),
        agent_id: paths.agent_id.clone(),
        ts_ms: ts,
        working_memory,
        thread_head,
        parent_branch,
        path: Some(path.clone()),
    };
    let body = serde_json::to_vec_pretty(&cp)
        .map_err(|e| HostError::json(path.clone(), e))?;
    std::fs::write(&path, body).map_err(|e| HostError::io(&path, e))?;
    Ok(cp)
}

pub fn latest_checkpoint(paths: &AgentPaths, branch_id: &str) -> HostResult<Option<Checkpoint>> {
    let dir = paths.checkpoints_dir().join(branch_id);
    if !dir.is_dir() {
        return Ok(None);
    }
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .map_err(|e| HostError::io(&dir, e))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();
    entries.sort_by_key(|e| e.file_name());
    let last = match entries.pop() {
        Some(e) => e.path(),
        None => return Ok(None),
    };
    let bytes = std::fs::read(&last).map_err(|e| HostError::io(&last, e))?;
    let mut cp: Checkpoint = serde_json::from_slice(&bytes)
        .map_err(|e| HostError::json(last.clone(), e))?;
    cp.path = Some(last);
    Ok(Some(cp))
}

pub fn list_checkpoints(paths: &AgentPaths, branch_id: &str) -> HostResult<Vec<PathBuf>> {
    let dir = paths.checkpoints_dir().join(branch_id);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .map_err(|e| HostError::io(&dir, e))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
        .map(|e| e.path())
        .collect();
    entries.sort();
    Ok(entries)
}

pub fn list_branches(paths: &AgentPaths) -> HostResult<Vec<String>> {
    let dir = paths.checkpoints_dir();
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|e| HostError::io(&dir, e))? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            if let Ok(name) = entry.file_name().into_string() {
                out.push(name);
            }
        }
    }
    out.sort();
    Ok(out)
}

pub fn fork_branch(paths: &AgentPaths, source_branch: &str, new_branch: &str) -> HostResult<Checkpoint> {
    let source = latest_checkpoint(paths, source_branch)?.ok_or_else(|| {
        HostError::Branching(format!("no checkpoint to fork from in `{source_branch}`"))
    })?;
    write_checkpoint(
        paths,
        new_branch,
        source.working_memory.clone(),
        source.thread_head.clone(),
        Some(source_branch.to_string()),
    )
}

pub fn current_branch(paths: &AgentPaths) -> HostResult<String> {
    let p = paths.checkpoints_dir().join("CURRENT");
    if !p.is_file() {
        return Ok(DEFAULT_BRANCH.to_string());
    }
    let text = std::fs::read_to_string(&p).map_err(|e| HostError::io(&p, e))?;
    Ok(text.trim().to_string())
}

pub fn switch_branch(paths: &AgentPaths, branch_id: &str) -> HostResult<()> {
    std::fs::create_dir_all(paths.checkpoints_dir())
        .map_err(|e| HostError::io(paths.checkpoints_dir(), e))?;
    let p = paths.checkpoints_dir().join("CURRENT");
    std::fs::write(&p, branch_id).map_err(|e| HostError::io(&p, e))?;
    Ok(())
}

pub fn delete_branch(paths: &AgentPaths, branch_id: &str, force: bool) -> HostResult<()> {
    if branch_id == DEFAULT_BRANCH && !force {
        return Err(HostError::Branching("refusing to delete `main` without force".into()));
    }
    let dir = paths.checkpoints_dir().join(branch_id);
    if dir.is_dir() {
        std::fs::remove_dir_all(&dir).map_err(|e| HostError::io(&dir, e))?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct BranchDiff {
    pub added_keys: Vec<String>,
    pub removed_keys: Vec<String>,
    pub changed_keys: Vec<ChangedKey>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChangedKey {
    pub key: String,
    pub a: serde_json::Value,
    pub b: serde_json::Value,
}

pub fn diff_branches(paths: &AgentPaths, a: &str, b: &str) -> HostResult<BranchDiff> {
    let ca = latest_checkpoint(paths, a)?
        .ok_or_else(|| HostError::Branching(format!("no checkpoint in branch `{a}`")))?;
    let cb = latest_checkpoint(paths, b)?
        .ok_or_else(|| HostError::Branching(format!("no checkpoint in branch `{b}`")))?;
    let empty = serde_json::Map::new();
    let ma = ca.working_memory.as_object().unwrap_or(&empty);
    let mb = cb.working_memory.as_object().unwrap_or(&empty);
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();
    for (k, va) in ma {
        match mb.get(k) {
            None => removed.push(k.clone()),
            Some(vb) if vb == va => {}
            Some(vb) => changed.push(ChangedKey { key: k.clone(), a: va.clone(), b: vb.clone() }),
        }
    }
    for (k, _) in mb {
        if !ma.contains_key(k) {
            added.push(k.clone());
        }
    }
    added.sort();
    removed.sort();
    changed.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(BranchDiff { added_keys: added, removed_keys: removed, changed_keys: changed })
}

pub fn prune_branch(paths: &AgentPaths, branch_id: &str, keep: usize) -> HostResult<usize> {
    let dir = paths.checkpoints_dir().join(branch_id);
    if !dir.is_dir() {
        return Ok(0);
    }
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .map_err(|e| HostError::io(&dir, e))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    entries.sort();
    let mut removed = 0;
    while entries.len() > keep {
        if let Some(p) = entries.first().cloned() {
            if std::fs::remove_file(&p).is_ok() {
                removed += 1;
            }
            entries.remove(0);
        } else {
            break;
        }
    }
    Ok(removed)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::HostPaths;
    use std::path::Path;
    use tempfile::tempdir;

    fn paths(root: &Path) -> AgentPaths {
        HostPaths::new(root).agent("alpha")
    }

    #[test]
    fn write_and_fork() {
        let tmp = tempdir().unwrap();
        let p = paths(tmp.path());
        write_checkpoint(&p, "main", serde_json::json!({"counter": 1}), None, None).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        write_checkpoint(&p, "main", serde_json::json!({"counter": 2}), None, None).unwrap();
        fork_branch(&p, "main", "bench").unwrap();
        let bs = list_branches(&p).unwrap();
        assert!(bs.contains(&"main".to_string()));
        assert!(bs.contains(&"bench".to_string()));
    }

    #[test]
    fn diff_keys() {
        let tmp = tempdir().unwrap();
        let p = paths(tmp.path());
        write_checkpoint(&p, "main", serde_json::json!({"a": 1, "b": 2}), None, None).unwrap();
        write_checkpoint(&p, "bench", serde_json::json!({"a": 1, "c": 3}), None, None).unwrap();
        let d = diff_branches(&p, "main", "bench").unwrap();
        assert_eq!(d.added_keys, vec!["c"]);
        assert_eq!(d.removed_keys, vec!["b"]);
    }
}
