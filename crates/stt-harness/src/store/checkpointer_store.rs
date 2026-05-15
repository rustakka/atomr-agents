//! `ConversationStore` backed by `crates/state`'s `Checkpointer`.
//!
//! This is the "configured persistence provider" path: a conversation
//! is stored as a `Snapshot` keyed
//! `{ workflow_id: "stt-harness", run_id: <conversation id>, super_step: <revision> }`,
//! so it lands in whichever `Checkpointer` backend the deployment wired
//! up — `InMemoryCheckpointer` by default, SQLite or Postgres behind
//! their feature flags.
//!
//! `Checkpointer` has no "list all runs" call, so this store keeps an
//! explicit index snapshot (run id `_index`) tracking the live
//! conversation ids. Deletes are tombstones — a snapshot whose
//! `values` carries `{"deleted": true}` — since `Checkpointer` has no
//! delete.
//!
//! Each `put` writes a new revision, so a speaker rename
//! (read-modify-write via [`ConversationStore::rename_speaker`])
//! produces a fresh checkpoint and survives a restart.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentError, RunId, Value, WorkflowId};
use atomr_agents_state::{CheckpointKey, Checkpointer, Snapshot};

use crate::conversation::SttConversation;
use crate::error::{Result, SttHarnessError};
use crate::store::{ConversationStore, ConversationSummary};

/// Synthetic workflow id under which all STT conversations are filed.
const WORKFLOW: &str = "stt-harness";
/// Synthetic run id for the conversation-id index snapshot.
const INDEX_RUN: &str = "_index";

/// A [`ConversationStore`] that routes through a `Checkpointer`.
pub struct CheckpointerConversationStore {
    checkpointer: Arc<dyn Checkpointer>,
}

impl CheckpointerConversationStore {
    /// Wrap any configured `Checkpointer`.
    pub fn new(checkpointer: Arc<dyn Checkpointer>) -> Self {
        Self { checkpointer }
    }

    fn wf() -> WorkflowId {
        WorkflowId::from(WORKFLOW)
    }

    /// Next free `super_step` for a run (its current latest + 1).
    async fn next_step(&self, run: &RunId) -> Result<u64> {
        let latest = self
            .checkpointer
            .latest(&Self::wf(), run)
            .await
            .map_err(persist_err)?;
        Ok(latest.map(|s| s.key.super_step + 1).unwrap_or(0))
    }

    /// Append a snapshot under `run`.
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
                label: label.to_string(),
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
        values.insert("ids".to_string(), serde_json::to_value(ids)?);
        self.save_values(RunId::from(INDEX_RUN), "index", values).await
    }
}

#[async_trait]
impl ConversationStore for CheckpointerConversationStore {
    async fn put(&self, conv: &SttConversation) -> Result<()> {
        let mut values = HashMap::new();
        values.insert("conversation".to_string(), serde_json::to_value(conv)?);
        self.save_values(RunId::from(conv.id.as_str()), "conversation", values)
            .await?;

        let mut ids = self.load_index().await?;
        if !ids.iter().any(|i| i == &conv.id) {
            ids.push(conv.id.clone());
            self.save_index(ids).await?;
        }
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<SttConversation>> {
        let snap = self
            .checkpointer
            .latest(&Self::wf(), &RunId::from(id))
            .await
            .map_err(persist_err)?;
        let Some(snap) = snap else {
            return Ok(None);
        };
        if snap
            .values
            .get("deleted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return Ok(None);
        }
        match snap.values.get("conversation") {
            Some(v) => Ok(Some(serde_json::from_value(v.clone())?)),
            None => Ok(None),
        }
    }

    async fn list(&self) -> Result<Vec<ConversationSummary>> {
        let ids = self.load_index().await?;
        let mut rows = Vec::new();
        for id in ids {
            if let Some(conv) = self.get(&id).await? {
                rows.push(ConversationSummary::of(&conv));
            }
        }
        rows.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(rows)
    }

    async fn delete(&self, id: &str) -> Result<()> {
        let mut values = HashMap::new();
        values.insert("deleted".to_string(), Value::Bool(true));
        self.save_values(RunId::from(id), "deleted", values).await?;

        let ids: Vec<String> = self.load_index().await?.into_iter().filter(|i| i != id).collect();
        self.save_index(ids).await?;
        Ok(())
    }
}

fn persist_err(e: AgentError) -> SttHarnessError {
    SttHarnessError::Persistence(e.to_string())
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_state::InMemoryCheckpointer;

    #[tokio::test]
    async fn speaker_label_survives_reload_through_checkpointer() {
        let checkpointer: Arc<dyn Checkpointer> = Arc::new(InMemoryCheckpointer::new());
        let store = CheckpointerConversationStore::new(checkpointer.clone());

        let mut conv = SttConversation::new("call-7");
        conv.commit_segment(atomr_agents_stt_core::Segment {
            text: "hi".into(),
            start_ms: 0,
            end_ms: 0,
            words: vec![],
            speaker: Some(atomr_agents_stt_core::SpeakerTag { id: 0, label: None }),
            confidence: None,
        });
        store.put(&conv).await.unwrap();

        store
            .rename_speaker("call-7", 0, "Alice".into())
            .await
            .unwrap()
            .unwrap();

        // A brand-new store over the *same* checkpointer (a "restart")
        // still sees the rename.
        let reopened = CheckpointerConversationStore::new(checkpointer);
        let reloaded = reopened.get("call-7").await.unwrap().unwrap();
        assert_eq!(reloaded.effective_label(0), "Alice");

        let listed = reopened.list().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "call-7");

        reopened.delete("call-7").await.unwrap();
        assert!(reopened.get("call-7").await.unwrap().is_none());
        assert!(reopened.list().await.unwrap().is_empty());
    }
}
