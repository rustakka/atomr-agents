//! Pluggable checkpointer.
//!
//! `InMemoryCheckpointer` is the default. SQLite + Postgres backends
//! land in Phase R17 behind feature flags.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{Result, RunId, Value, WorkflowId};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CheckpointKey {
    pub workflow_id: WorkflowId,
    pub run_id: RunId,
    pub super_step: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointMeta {
    pub workflow_id: WorkflowId,
    pub run_id: RunId,
    pub super_step: u64,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub key: CheckpointKey,
    pub values: HashMap<String, Value>,
    /// Optional human label describing what produced this snapshot
    /// (step name, "interrupt", etc.).
    #[serde(default)]
    pub label: String,
    pub timestamp_ms: i64,
}

#[async_trait]
pub trait Checkpointer: Send + Sync + 'static {
    async fn save(&self, snapshot: Snapshot) -> Result<()>;
    async fn load(&self, key: &CheckpointKey) -> Result<Option<Snapshot>>;
    /// Returns the latest snapshot for a `(workflow, run)` pair.
    async fn latest(&self, workflow_id: &WorkflowId, run_id: &RunId) -> Result<Option<Snapshot>>;
    async fn list(&self, workflow_id: &WorkflowId, run_id: &RunId) -> Result<Vec<CheckpointMeta>>;
    /// Create a new run that diverges from an existing checkpoint
    /// with an optional set of state edits applied at the fork point.
    async fn fork(&self, from: &CheckpointKey, edits: Vec<(String, Value)>) -> Result<RunId>;
}

#[derive(Default, Clone)]
pub struct InMemoryCheckpointer {
    inner: Arc<RwLock<Vec<Snapshot>>>,
}

impl InMemoryCheckpointer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.read().is_empty()
    }
}

#[async_trait]
impl Checkpointer for InMemoryCheckpointer {
    async fn save(&self, snapshot: Snapshot) -> Result<()> {
        self.inner.write().push(snapshot);
        Ok(())
    }

    async fn load(&self, key: &CheckpointKey) -> Result<Option<Snapshot>> {
        Ok(self
            .inner
            .read()
            .iter()
            .find(|s| {
                s.key.workflow_id.as_str() == key.workflow_id.as_str()
                    && s.key.run_id.as_str() == key.run_id.as_str()
                    && s.key.super_step == key.super_step
            })
            .cloned())
    }

    async fn latest(&self, workflow_id: &WorkflowId, run_id: &RunId) -> Result<Option<Snapshot>> {
        let g = self.inner.read();
        Ok(g.iter()
            .filter(|s| {
                s.key.workflow_id.as_str() == workflow_id.as_str() && s.key.run_id.as_str() == run_id.as_str()
            })
            .max_by_key(|s| s.key.super_step)
            .cloned())
    }

    async fn list(&self, workflow_id: &WorkflowId, run_id: &RunId) -> Result<Vec<CheckpointMeta>> {
        Ok(self
            .inner
            .read()
            .iter()
            .filter(|s| {
                s.key.workflow_id.as_str() == workflow_id.as_str() && s.key.run_id.as_str() == run_id.as_str()
            })
            .map(|s| CheckpointMeta {
                workflow_id: s.key.workflow_id.clone(),
                run_id: s.key.run_id.clone(),
                super_step: s.key.super_step,
                timestamp_ms: s.timestamp_ms,
            })
            .collect())
    }

    async fn fork(&self, from: &CheckpointKey, edits: Vec<(String, Value)>) -> Result<RunId> {
        let snap = self.load(from).await?.ok_or_else(|| {
            atomr_agents_core::AgentError::Internal(format!(
                "fork: source checkpoint {}#{} not found",
                from.run_id.as_str(),
                from.super_step
            ))
        })?;
        let new_run = RunId::new();
        let mut values = snap.values.clone();
        for (k, v) in edits {
            values.insert(k, v);
        }
        self.save(Snapshot {
            key: CheckpointKey {
                workflow_id: snap.key.workflow_id.clone(),
                run_id: new_run.clone(),
                super_step: snap.key.super_step,
            },
            values,
            label: format!("fork-of:{}", from.run_id.as_str()),
            timestamp_ms: chrono_now_ms(),
        })
        .await?;
        Ok(new_run)
    }
}

fn chrono_now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snap(wf: &str, run: &str, step: u64, label: &str, kvs: Vec<(&str, Value)>) -> Snapshot {
        let mut values = HashMap::new();
        for (k, v) in kvs {
            values.insert(k.into(), v);
        }
        Snapshot {
            key: CheckpointKey {
                workflow_id: WorkflowId::from(wf),
                run_id: RunId::from(run),
                super_step: step,
            },
            values,
            label: label.into(),
            timestamp_ms: chrono_now_ms(),
        }
    }

    #[tokio::test]
    async fn save_and_latest() {
        let c = InMemoryCheckpointer::new();
        c.save(snap("wf", "r", 0, "init", vec![("messages", json!([]))]))
            .await
            .unwrap();
        c.save(snap(
            "wf",
            "r",
            2,
            "after",
            vec![("messages", json!([{"id": "m1"}]))],
        ))
        .await
        .unwrap();
        let latest = c
            .latest(&WorkflowId::from("wf"), &RunId::from("r"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest.key.super_step, 2);
        assert_eq!(latest.values["messages"][0]["id"], "m1");
    }

    #[tokio::test]
    async fn fork_creates_new_run_with_edits() {
        let c = InMemoryCheckpointer::new();
        c.save(snap("wf", "main", 1, "before-fork", vec![("a", json!(1))]))
            .await
            .unwrap();
        let new_run = c
            .fork(
                &CheckpointKey {
                    workflow_id: WorkflowId::from("wf"),
                    run_id: RunId::from("main"),
                    super_step: 1,
                },
                vec![("a".into(), json!(99))],
            )
            .await
            .unwrap();
        let forked = c
            .latest(&WorkflowId::from("wf"), &new_run)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(forked.values["a"], json!(99));
        assert!(forked.label.starts_with("fork-of:main"));
    }

    #[tokio::test]
    async fn list_returns_meta_in_order() {
        let c = InMemoryCheckpointer::new();
        for step in [0u64, 1, 2, 3] {
            c.save(snap("wf", "r", step, "step", vec![])).await.unwrap();
        }
        let metas = c.list(&WorkflowId::from("wf"), &RunId::from("r")).await.unwrap();
        assert_eq!(metas.len(), 4);
    }
}
