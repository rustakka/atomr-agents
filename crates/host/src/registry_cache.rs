//! M11 — Disk-cached registry pulls.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{HostError, HostResult};
use crate::layout::HostPaths;

pub const ARTIFACT_KINDS: &[&str] = &[
    "tool_set", "skill", "persona", "agent", "workflow", "harness", "channel",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedArtifact {
    pub kind: String,
    pub id: String,
    pub version: String,
    pub payload: serde_json::Value,
    #[serde(default)]
    pub cached_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

impl CachedArtifact {
    pub fn slug(&self) -> String {
        format!("{}:{}@{}", self.kind, self.id, self.version)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SlugRef {
    pub kind: String,
    pub id: String,
    pub version: Option<String>,
}

pub fn parse_slug(slug: &str) -> HostResult<SlugRef> {
    let (kind, rest) = slug
        .split_once(':')
        .ok_or_else(|| HostError::Registry(format!("invalid slug `{slug}` (expected kind:id@version)")))?;
    let (id, version) = match rest.split_once('@') {
        Some((i, v)) => (i.to_string(), Some(v.to_string())),
        None => (rest.to_string(), None),
    };
    if !ARTIFACT_KINDS.contains(&kind) {
        return Err(HostError::Registry(format!(
            "unknown kind `{kind}` (valid: {ARTIFACT_KINDS:?})"
        )));
    }
    if id.is_empty() {
        return Err(HostError::Registry(format!("empty id in `{slug}`")));
    }
    Ok(SlugRef { kind: kind.into(), id, version })
}

fn artifact_path(host: &HostPaths, kind: &str, id: &str, version: &str) -> PathBuf {
    host.registry_dir().join(kind).join(id).join(format!("{version}.json"))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn cache_artifact(
    host: &HostPaths,
    kind: &str,
    id: &str,
    version: &str,
    payload: serde_json::Value,
) -> HostResult<CachedArtifact> {
    let path = artifact_path(host, kind, id, version);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| HostError::io(parent, e))?;
    }
    let art = CachedArtifact {
        kind: kind.into(),
        id: id.into(),
        version: version.into(),
        payload,
        cached_at_ms: now_ms(),
        path: Some(path.clone()),
    };
    let body = serde_json::to_vec_pretty(&art).map_err(|e| HostError::json(path.clone(), e))?;
    std::fs::write(&path, body).map_err(|e| HostError::io(&path, e))?;
    Ok(art)
}

pub fn resolve_artifact(
    host: &HostPaths,
    kind: &str,
    id: &str,
    version: &str,
) -> HostResult<CachedArtifact> {
    let path = artifact_path(host, kind, id, version);
    if !path.is_file() {
        return Err(HostError::Registry(format!(
            "no cached artifact at {}",
            path.display()
        )));
    }
    let bytes = std::fs::read(&path).map_err(|e| HostError::io(&path, e))?;
    let mut art: CachedArtifact =
        serde_json::from_slice(&bytes).map_err(|e| HostError::json(path.clone(), e))?;
    art.path = Some(path);
    Ok(art)
}

pub fn delete_artifact(host: &HostPaths, kind: &str, id: &str, version: &str) -> HostResult<bool> {
    let path = artifact_path(host, kind, id, version);
    if path.is_file() {
        std::fs::remove_file(&path).map_err(|e| HostError::io(&path, e))?;
        return Ok(true);
    }
    Ok(false)
}

pub fn list_artifacts(host: &HostPaths, kind: Option<&str>) -> HostResult<Vec<CachedArtifact>> {
    let dir = host.registry_dir();
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let kinds: Vec<String> = match kind {
        Some(k) => vec![k.to_string()],
        None => ARTIFACT_KINDS.iter().map(|s| s.to_string()).collect(),
    };
    for k in kinds {
        let kdir = dir.join(&k);
        if !kdir.is_dir() {
            continue;
        }
        for id_entry in std::fs::read_dir(&kdir).map_err(|e| HostError::io(&kdir, e))? {
            let id_entry = match id_entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let id_path = id_entry.path();
            if !id_path.is_dir() {
                continue;
            }
            let id = match id_path.file_name().and_then(|n| n.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            for v_entry in std::fs::read_dir(&id_path).map_err(|e| HostError::io(&id_path, e))? {
                let v_entry = match v_entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                let v_path = v_entry.path();
                if v_path.extension().and_then(|s| s.to_str()) != Some("json") {
                    continue;
                }
                let v = match v_path.file_stem().and_then(|n| n.to_str()) {
                    Some(s) => s.to_string(),
                    None => continue,
                };
                match resolve_artifact(host, &k, &id, &v) {
                    Ok(a) => out.push(a),
                    Err(_) => continue,
                }
            }
        }
    }
    out.sort_by_key(|a| a.slug());
    Ok(out)
}

/// Pull an artifact via the supplied lookup function. The function is
/// duck-typed so the caller can use any `Registry`-shaped object —
/// production code wires in the native `Registry`; tests can pass a
/// closure.
pub fn pull_artifact<F>(
    host: &HostPaths,
    kind: &str,
    id: &str,
    version: Option<&str>,
    fetch: F,
) -> HostResult<CachedArtifact>
where
    F: Fn(&str, &str, Option<&str>) -> HostResult<(String, serde_json::Value)>,
{
    let (resolved_version, payload) = fetch(kind, id, version)?;
    cache_artifact(host, kind, id, &resolved_version, payload)
}

/// Cross-check the on-disk cache against the supplied resolver. Returns
/// the diff list — `[]` when consistent.
pub fn verify_cache<F>(host: &HostPaths, fetch: F) -> HostResult<Vec<(CachedArtifact, &'static str)>>
where
    F: Fn(&str, &str, &str) -> HostResult<Option<serde_json::Value>>,
{
    let mut diffs = Vec::new();
    for art in list_artifacts(host, None)? {
        match fetch(&art.kind, &art.id, &art.version)? {
            None => diffs.push((art, "missing")),
            Some(payload) if payload != art.payload => diffs.push((art, "mismatch")),
            _ => {}
        }
    }
    Ok(diffs)
}

pub fn ensure_dir(host: &HostPaths) -> HostResult<PathBuf> {
    let dir = host.registry_dir();
    std::fs::create_dir_all(&dir).map_err(|e| HostError::io(&dir, e))?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn cache_and_resolve_roundtrip() {
        let tmp = tempdir().unwrap();
        let host = HostPaths::new(tmp.path());
        cache_artifact(&host, "skill", "summarize", "0.1.0", serde_json::json!({"n":"S"})).unwrap();
        let art = resolve_artifact(&host, "skill", "summarize", "0.1.0").unwrap();
        assert_eq!(art.slug(), "skill:summarize@0.1.0");
        assert_eq!(art.payload.get("n").and_then(|v| v.as_str()), Some("S"));
    }

    #[test]
    fn parse_slug_basic() {
        let s = parse_slug("skill:summarize@0.1.0").unwrap();
        assert_eq!(s.kind, "skill");
        assert_eq!(s.id, "summarize");
        assert_eq!(s.version.as_deref(), Some("0.1.0"));
    }

    #[test]
    fn parse_slug_rejects_unknown_kind() {
        assert!(parse_slug("bogus:foo@1").is_err());
    }
}
