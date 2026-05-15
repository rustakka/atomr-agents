//! `MeetingsStore` backed by `crates/state`'s `Checkpointer`.
//!
//! Mirrors `CheckpointerConversationStore`: an analysis is stored as a
//! `Snapshot` keyed
//! `{ workflow_id: "meetings-harness", run_id: <conversation id>, super_step: <rev> }`,
//! so it lands in whichever `Checkpointer` backend the deployment wired
//! up. The `run_id` is the **same as the STT transcript's** — both
//! records join naturally under the same id in the same store.
//!
//! An index snapshot under `run_id = "_index"` tracks live ids;
//! deletes are tombstones (snapshot with `values["deleted"] = true`).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentError, RunId, Value, WorkflowId};
use atomr_agents_state::{CheckpointKey, Checkpointer, Snapshot};

use crate::analysis::MeetingAnalysis;
use crate::error::{MeetingsHarnessError, Result};
use crate::store::{MeetingsStore, MeetingsSummary};

const WORKFLOW: &str = "meetings-harness";
const INDEX_RUN: &str = "_index";

/// A [`MeetingsStore`] that routes through a `Checkpointer`.
pub struct CheckpointerMeetingsStore {
    checkpointer: Arc<dyn Checkpointer>,
}

impl CheckpointerMeetingsStore {
    pub fn new(checkpointer: Arc<dyn Checkpointer>) -> Self {
        Self { checkpointer }
    }

    fn wf() -> WorkflowId {
        WorkflowId::from(WORKFLOW)
    }

    async fn next_step(&self, run: &RunId) -> Result<u64> {
        let latest = self
            .checkpointer
            .latest(&Self::wf(), run)
            .await
            .map_err(persist_err)?;
        Ok(latest.map(|s| s.key.super_step + 1).unwrap_or(0))
    }

    async fn save_values(&self, run: RunId, label: &str, values: HashMap<String, Value>) -> Result<()> {
        let step = self.next_step(&run).await?;
        self.checkpointer
            .save(Snapshot {
                key: CheckpointKey {
                    workflow_id: Self::wf(),
                    run_id: run,
                    super_step: step,
                },
                values,
                label: label.into(),
                timestamp_ms: now_ms(),
            })
            .await
            .map_err(persist_err)
    }

    async fn load_index(&self) -> Result<Vec<String>> {
        let snap = self
            .checkpointer
            .latest(&Self::wf(), &RunId::from(INDEX_RUN))
            .await
            .map_err(persist_err)?;
        match snap {
            None => Ok(Vec::new()),
            Some(s) => Ok(s
                .values
                .get("ids")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default()),
        }
    }

    async fn save_index(&self, ids: Vec<String>) -> Result<()> {
        let mut values = HashMap::new();
        values.insert("ids".into(), serde_json::to_value(ids)?);
        self.save_values(RunId::from(INDEX_RUN), "index", values).await
    }
}

#[async_trait]
impl MeetingsStore for CheckpointerMeetingsStore {
    async fn put(&self, analysis: &MeetingAnalysis) -> Result<()> {
        let mut values = HashMap::new();
        values.insert("analysis".into(), serde_json::to_value(analysis)?);
        self.save_values(RunId::from(analysis.id.as_str()), "analysis", values)
            .await?;
        let mut ids = self.load_index().await?;
        if !ids.iter().any(|i| i == &analysis.id) {
            ids.push(analysis.id.clone());
            self.save_index(ids).await?;
        }
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<MeetingAnalysis>> {
        let snap = self
            .checkpointer
            .latest(&Self::wf(), &RunId::from(id))
            .await
            .map_err(persist_err)?;
        let Some(snap) = snap else { return Ok(None) };
        if snap
            .values
            .get("deleted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return Ok(None);
        }
        match snap.values.get("analysis") {
            Some(v) => Ok(Some(serde_json::from_value(v.clone())?)),
            None => Ok(None),
        }
    }

    async fn list(&self) -> Result<Vec<MeetingsSummary>> {
        let ids = self.load_index().await?;
        let mut rows = Vec::new();
        for id in ids {
            if let Some(a) = self.get(&id).await? {
                rows.push(MeetingsSummary::of(&a));
            }
        }
        rows.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(rows)
    }

    async fn delete(&self, id: &str) -> Result<()> {
        let mut values = HashMap::new();
        values.insert("deleted".into(), Value::Bool(true));
        self.save_values(RunId::from(id), "deleted", values).await?;
        let ids: Vec<String> = self.load_index().await?.into_iter().filter(|i| i != id).collect();
        self.save_index(ids).await?;
        Ok(())
    }
}

fn persist_err(e: AgentError) -> MeetingsHarnessError {
    MeetingsHarnessError::Persistence(e.to_string())
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_state::InMemoryCheckpointer;

    #[tokio::test]
    async fn round_trips_through_checkpointer_and_lists() {
        let cp: Arc<dyn Checkpointer> = Arc::new(InMemoryCheckpointer::new());
        let store = CheckpointerMeetingsStore::new(cp.clone());
        let mut a = MeetingAnalysis::new("call-7");
        a.title = Some("Hello".into());
        store.put(&a).await.unwrap();

        let reopened = CheckpointerMeetingsStore::new(cp);
        let back = reopened.get("call-7").await.unwrap().unwrap();
        assert_eq!(back.title.as_deref(), Some("Hello"));

        let listed = reopened.list().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "call-7");

        reopened.delete("call-7").await.unwrap();
        assert!(reopened.get("call-7").await.unwrap().is_none());
        assert!(reopened.list().await.unwrap().is_empty());
    }
}
