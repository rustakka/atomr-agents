//! Provenance capture for record-and-replay determinism (FR-1).
//!
//! A [`StepRecord`] captures, per `(workflow, run, super_step)`, the raw
//! model completion and the verbatim tool returns that produced a state
//! transition — enough to re-execute the step in replay mode with **zero
//! provider calls** and **zero side-effecting tool invocations**.
//!
//! Records are provider-agnostic here (primitives + JSON) so the state
//! crate stays dependency-light; the agent crate's `ReplayProvider`
//! reconstructs a `TurnResult` from an [`InferenceRecord`].

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{Result, RunId, Value, WorkflowId};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::checkpointer::{CheckpointKey, Checkpointer, Snapshot};

/// Token accounting captured for a step.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageRecord {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    #[serde(default)]
    pub reasoning_tokens: u32,
    #[serde(default)]
    pub cached_tokens: u32,
}

/// The model side of a step: what provider/model produced what
/// completion, with hashes that pin the request and detect drift.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InferenceRecord {
    pub provider: String,
    pub model_id: String,
    pub model_version: String,
    pub params_hash: String,
    pub prompt_hash: String,
    /// The raw assistant text completion, verbatim.
    pub raw_completion: String,
    /// `FinishReason` serialized as a string (e.g. `"stop"`,
    /// `"tool_calls"`), or `None`.
    #[serde(default)]
    pub finish_reason: Option<String>,
    pub usage: UsageRecord,
    /// Parsed tool calls the model emitted, each a serialized
    /// `ParsedToolCall`. Replayed verbatim so replay is byte-identical.
    #[serde(default)]
    pub tool_calls: Vec<Value>,
}

/// A single tool dispatch within a step: the verbatim return plus
/// whether the tool has external side effects (so replay can serve the
/// recorded return instead of re-invoking, e.g. a broker order).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub tool: String,
    pub args_hash: String,
    /// The verbatim `ToolReturn`, serialized.
    pub ret: Value,
    pub is_side_effecting: bool,
}

/// Everything recorded for one `(workflow, run, super_step)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepRecord {
    pub key: CheckpointKey,
    #[serde(default)]
    pub inference: Option<InferenceRecord>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCallRecord>,
    /// The state snapshot after this step (verbatim), so replay is
    /// self-contained without a separate checkpoint load.
    #[serde(default)]
    pub state_snapshot: HashMap<String, Value>,
}

impl StepRecord {
    pub fn new(key: CheckpointKey) -> Self {
        Self {
            key,
            inference: None,
            tool_calls: Vec::new(),
            state_snapshot: HashMap::new(),
        }
    }
    pub fn with_inference(mut self, inf: InferenceRecord) -> Self {
        self.inference = Some(inf);
        self
    }
    pub fn with_tool_call(mut self, tc: ToolCallRecord) -> Self {
        self.tool_calls.push(tc);
        self
    }
    pub fn with_state(mut self, state: HashMap<String, Value>) -> Self {
        self.state_snapshot = state;
        self
    }
}

/// Persists and retrieves [`StepRecord`]s. Mirrors [`Checkpointer`] keying
/// so records and checkpoints share `(workflow, run, super_step)`.
#[async_trait]
pub trait StepRecordStore: Send + Sync + 'static {
    async fn save_record(&self, record: StepRecord) -> Result<()>;
    async fn load_record(&self, key: &CheckpointKey) -> Result<Option<StepRecord>>;
    /// Records for a run, ordered ascending by `super_step`.
    async fn list_records(&self, workflow_id: &WorkflowId, run_id: &RunId) -> Result<Vec<StepRecord>>;
}

/// In-memory record store (default / tests).
#[derive(Default, Clone)]
pub struct InMemoryStepRecordStore {
    inner: Arc<RwLock<Vec<StepRecord>>>,
}

impl InMemoryStepRecordStore {
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
impl StepRecordStore for InMemoryStepRecordStore {
    async fn save_record(&self, record: StepRecord) -> Result<()> {
        let mut g = self.inner.write();
        if let Some(slot) = g.iter_mut().find(|r| same_key(&r.key, &record.key)) {
            *slot = record;
        } else {
            g.push(record);
        }
        Ok(())
    }

    async fn load_record(&self, key: &CheckpointKey) -> Result<Option<StepRecord>> {
        Ok(self.inner.read().iter().find(|r| same_key(&r.key, key)).cloned())
    }

    async fn list_records(&self, workflow_id: &WorkflowId, run_id: &RunId) -> Result<Vec<StepRecord>> {
        let mut v: Vec<StepRecord> = self
            .inner
            .read()
            .iter()
            .filter(|r| {
                r.key.workflow_id.as_str() == workflow_id.as_str() && r.key.run_id.as_str() == run_id.as_str()
            })
            .cloned()
            .collect();
        v.sort_by_key(|r| r.key.super_step);
        Ok(v)
    }
}

pub(crate) fn same_key(a: &CheckpointKey, b: &CheckpointKey) -> bool {
    a.workflow_id.as_str() == b.workflow_id.as_str()
        && a.run_id.as_str() == b.run_id.as_str()
        && a.super_step == b.super_step
}

/// Wraps any [`Checkpointer`] and additionally records a [`StepRecord`]
/// per step. State snapshots flow to the base checkpointer (unchanged
/// behavior); inference/tool provenance flows to the record store.
///
/// The runtime calls [`RecordingCheckpointer::record`] after a step to
/// capture the model completion + verbatim tool returns; replay then
/// reads those records instead of re-inferring / re-firing tools.
pub struct RecordingCheckpointer {
    base: Arc<dyn Checkpointer>,
    records: Arc<dyn StepRecordStore>,
}

impl RecordingCheckpointer {
    pub fn new(base: Arc<dyn Checkpointer>, records: Arc<dyn StepRecordStore>) -> Self {
        Self { base, records }
    }

    /// In-memory convenience: wrap an in-memory checkpointer + store.
    pub fn in_memory() -> Self {
        Self {
            base: Arc::new(crate::checkpointer::InMemoryCheckpointer::new()),
            records: Arc::new(InMemoryStepRecordStore::new()),
        }
    }

    /// Capture provenance for a step.
    pub async fn record(&self, record: StepRecord) -> Result<()> {
        self.records.save_record(record).await
    }

    /// Access the underlying record store (for replay construction).
    pub fn records(&self) -> Arc<dyn StepRecordStore> {
        self.records.clone()
    }
}

#[async_trait]
impl Checkpointer for RecordingCheckpointer {
    async fn save(&self, snapshot: Snapshot) -> Result<()> {
        self.base.save(snapshot).await
    }
    async fn load(&self, key: &CheckpointKey) -> Result<Option<Snapshot>> {
        self.base.load(key).await
    }
    async fn latest(&self, workflow_id: &WorkflowId, run_id: &RunId) -> Result<Option<Snapshot>> {
        self.base.latest(workflow_id, run_id).await
    }
    async fn list(
        &self,
        workflow_id: &WorkflowId,
        run_id: &RunId,
    ) -> Result<Vec<crate::checkpointer::CheckpointMeta>> {
        self.base.list(workflow_id, run_id).await
    }
    async fn fork(&self, from: &CheckpointKey, edits: Vec<(String, Value)>) -> Result<RunId> {
        self.base.fork(from, edits).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key(wf: &str, run: &str, step: u64) -> CheckpointKey {
        CheckpointKey {
            workflow_id: WorkflowId::from(wf),
            run_id: RunId::from(run),
            super_step: step,
        }
    }

    #[tokio::test]
    async fn record_store_roundtrip_and_order() {
        let store = InMemoryStepRecordStore::new();
        for step in [2u64, 0, 1] {
            store
                .save_record(
                    StepRecord::new(key("wf", "r", step)).with_inference(InferenceRecord {
                        provider: "anthropic".into(),
                        model_id: "claude".into(),
                        model_version: "1".into(),
                        params_hash: "p".into(),
                        prompt_hash: "h".into(),
                        raw_completion: format!("step {step}"),
                        finish_reason: Some("stop".into()),
                        usage: UsageRecord {
                            prompt_tokens: 1,
                            completion_tokens: 1,
                            ..Default::default()
                        },
                        tool_calls: vec![],
                    }),
                )
                .await
                .unwrap();
        }
        let recs = store
            .list_records(&WorkflowId::from("wf"), &RunId::from("r"))
            .await
            .unwrap();
        assert_eq!(recs.len(), 3);
        assert_eq!(recs[0].key.super_step, 0);
        assert_eq!(recs[2].key.super_step, 2);

        let one = store.load_record(&key("wf", "r", 1)).await.unwrap().unwrap();
        assert_eq!(one.inference.unwrap().raw_completion, "step 1");
    }

    #[tokio::test]
    async fn recording_checkpointer_delegates_and_records() {
        let rec = RecordingCheckpointer::in_memory();
        // checkpoint save/load delegates to base
        rec.save(Snapshot {
            key: key("wf", "r", 0),
            values: HashMap::new(),
            label: "init".into(),
            timestamp_ms: 1,
        })
        .await
        .unwrap();
        assert!(rec.load(&key("wf", "r", 0)).await.unwrap().is_some());

        // record provenance
        rec.record(StepRecord::new(key("wf", "r", 0)).with_tool_call(ToolCallRecord {
            tool: "place_order".into(),
            args_hash: "a".into(),
            ret: json!({"ok": true}),
            is_side_effecting: true,
        }))
        .await
        .unwrap();
        let got = rec
            .records()
            .load_record(&key("wf", "r", 0))
            .await
            .unwrap()
            .unwrap();
        assert!(got.tool_calls[0].is_side_effecting);
    }
}
