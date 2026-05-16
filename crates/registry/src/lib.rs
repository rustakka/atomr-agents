//! Versioned artifact registry. Every artifact carries a SemVer; the
//! `Registry` stores them by `(id, version)` and `latest()` returns
//! the highest version. `publish_gated` requires an eval result that
//! beats a baseline before insertion.

use std::sync::Arc;

use atomr_agents_core::{AgentError, Result};
use dashmap::DashMap;
use semver::Version;
use serde::{Deserialize, Serialize};

/// Generic key — every artifact kind uses an `(id, version)` tuple.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArtifactKey {
    pub kind: ArtifactKind,
    pub id: String,
    pub version: Version,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    ToolSet,
    Skill,
    Persona,
    Agent,
    Workflow,
    Harness,
    HarnessSet,
    Channel,
    Avatar,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactRecord {
    pub kind: ArtifactKind,
    pub id: String,
    pub version: Version,
    /// Inline serialized payload. Avoids tying the registry to every
    /// artifact's concrete Rust type.
    pub payload: serde_json::Value,
    pub published_at_ms: i64,
    pub baseline_pass_rate: Option<f32>,
    pub current_pass_rate: Option<f32>,
}

#[derive(Default, Clone)]
pub struct Registry {
    inner: Arc<DashMap<(ArtifactKind, String, Version), Arc<ArtifactRecord>>>,
}

/// Outcome of an eval run, kept generic so callers can pass anything
/// that exposes a pass rate. We only need the numeric value.
pub struct EvalSummary {
    pub pass_rate: f32,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn publish(&self, record: ArtifactRecord) -> Arc<ArtifactRecord> {
        let key = (record.kind, record.id.clone(), record.version.clone());
        let arc = Arc::new(record);
        self.inner.insert(key, arc.clone());
        arc
    }

    /// Publish iff `current.pass_rate >= baseline.pass_rate - tolerance`.
    /// Errors with `PolicyDenied` if the regression check fails.
    pub fn publish_gated(
        &self,
        record: ArtifactRecord,
        baseline: Option<&EvalSummary>,
        current: &EvalSummary,
        tolerance: f32,
    ) -> Result<Arc<ArtifactRecord>> {
        if let Some(b) = baseline {
            if current.pass_rate + tolerance < b.pass_rate {
                return Err(AgentError::PolicyDenied(format!(
                    "regression: pass_rate {:.3} < baseline {:.3} - tol {:.3}",
                    current.pass_rate, b.pass_rate, tolerance
                )));
            }
        }
        let mut record = record;
        record.baseline_pass_rate = baseline.map(|b| b.pass_rate);
        record.current_pass_rate = Some(current.pass_rate);
        Ok(self.publish(record))
    }

    pub fn get(&self, kind: ArtifactKind, id: &str, version: &Version) -> Option<Arc<ArtifactRecord>> {
        self.inner
            .get(&(kind, id.to_string(), version.clone()))
            .map(|r| r.value().clone())
    }

    pub fn latest(&self, kind: ArtifactKind, id: &str) -> Option<Arc<ArtifactRecord>> {
        self.inner
            .iter()
            .filter(|r| r.key().0 == kind && r.key().1 == id)
            .map(|r| r.value().clone())
            .max_by(|a, b| a.version.cmp(&b.version))
    }

    pub fn list(&self, kind: ArtifactKind) -> Vec<Arc<ArtifactRecord>> {
        self.inner
            .iter()
            .filter(|r| r.key().0 == kind)
            .map(|r| r.value().clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(kind: ArtifactKind, id: &str, v: (u64, u64, u64)) -> ArtifactRecord {
        ArtifactRecord {
            kind,
            id: id.into(),
            version: Version::new(v.0, v.1, v.2),
            payload: serde_json::json!({"id": id}),
            published_at_ms: 0,
            baseline_pass_rate: None,
            current_pass_rate: None,
        }
    }

    #[test]
    fn publish_pin_and_latest() {
        let r = Registry::new();
        r.publish(record(ArtifactKind::ToolSet, "ts", (0, 1, 0)));
        r.publish(record(ArtifactKind::ToolSet, "ts", (0, 2, 0)));
        let latest = r.latest(ArtifactKind::ToolSet, "ts").unwrap();
        assert_eq!(latest.version, Version::new(0, 2, 0));
        let pinned = r
            .get(ArtifactKind::ToolSet, "ts", &Version::new(0, 1, 0))
            .unwrap();
        assert_eq!(pinned.version, Version::new(0, 1, 0));
    }

    #[test]
    fn gated_publish_blocks_regression() {
        let r = Registry::new();
        let baseline = EvalSummary { pass_rate: 0.95 };
        let current = EvalSummary { pass_rate: 0.50 };
        let res = r.publish_gated(
            record(ArtifactKind::Harness, "ch", (0, 1, 0)),
            Some(&baseline),
            &current,
            0.05,
        );
        assert!(res.is_err());
    }

    #[test]
    fn gated_publish_allows_no_regression() {
        let r = Registry::new();
        let baseline = EvalSummary { pass_rate: 0.95 };
        let current = EvalSummary { pass_rate: 0.93 };
        let res = r.publish_gated(
            record(ArtifactKind::Harness, "ch", (0, 1, 0)),
            Some(&baseline),
            &current,
            0.05,
        );
        let arc = res.unwrap();
        assert_eq!(arc.current_pass_rate, Some(0.93));
    }
}
