//! FR-2 — Deterministic replay sink: feed recorded decisions to an
//! external simulator without re-inference.
//!
//! hedgehog regression-tests a strategy by replaying a recorded LIVE run
//! against a deterministic SimulatedVenue (a discrete-event-simulation
//! twin) to confirm behaviour reproduces and to A/B a new model against
//! historical decisions. This needs the recorded *side-effecting* tool
//! decisions (e.g. broker order `Command`s) routed to an external simulator
//! **in order**, decoupled from the live broker tool — with no model or
//! tool invocation.
//!
//! [`replay_to_sink`] walks the [`StepRecordStore`] in `(workflow, run,
//! super_step)` order and emits each side-effecting [`ToolCallRecord`] to a
//! [`DecisionReplaySink`]. Pure (non-side-effecting) records are skipped —
//! they have no external effect to reproduce. Replay is resumable: it
//! accepts a starting cursor and returns the last emitted super-step, so an
//! interrupted replay continues deterministically.
//!
//! [`ChannelReplaySink`] adapts a tokio mpsc channel so a downstream
//! simulator can subscribe with backpressure.

use async_trait::async_trait;
use atomr_agents_core::Result;
use atomr_agents_state::{StepRecordStore, ToolCallRecord};

/// Identifies the recorded step a decision came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepRef {
    /// Workflow id.
    pub workflow: String,
    /// Run id.
    pub run: String,
    /// Super-step the decision was recorded at.
    pub super_step: u64,
}

/// Outbound contract that drives a non-agent simulator from recorded
/// decisions. `emit` is called once per side-effecting tool decision in
/// recorded order.
#[async_trait]
pub trait DecisionReplaySink: Send + Sync {
    /// Receive one recorded side-effecting decision.
    async fn emit(&self, step: StepRef, decision: &ToolCallRecord) -> Result<()>;
}

/// Walk recorded steps for `(workflow, run)` in ascending `super_step`
/// order and emit each **side-effecting** [`ToolCallRecord`] to `sink`,
/// performing NO model or tool invocation.
///
/// Resumable: only steps with `super_step >= from_cursor` are emitted.
/// Returns the highest super_step that produced an emission (the cursor to
/// resume *after*), or `from_cursor` if nothing was emitted.
///
/// At-least-once: re-running from a cursor re-emits that step's decisions,
/// so the downstream simulator must dedupe on `StepRef` if it requires
/// exactly-once. Ordering is stable across reruns because it follows the
/// recorded `super_step`.
pub async fn replay_to_sink(
    records: &dyn StepRecordStore,
    workflow: &str,
    run: &str,
    from_cursor: u64,
    sink: &dyn DecisionReplaySink,
) -> Result<u64> {
    use atomr_agents_core::{RunId, WorkflowId};

    let recs = records
        .list_records(&WorkflowId::from(workflow), &RunId::from(run))
        .await?;
    // list_records is already ascending by super_step (see StepRecordStore).
    let mut cursor = from_cursor;
    for rec in recs {
        if rec.key.super_step < from_cursor {
            continue;
        }
        let mut emitted_here = false;
        for tc in &rec.tool_calls {
            if !tc.is_side_effecting {
                continue; // pure decision: nothing to reproduce externally
            }
            sink.emit(
                StepRef {
                    workflow: workflow.to_string(),
                    run: run.to_string(),
                    super_step: rec.key.super_step,
                },
                tc,
            )
            .await?;
            emitted_here = true;
        }
        if emitted_here {
            cursor = rec.key.super_step;
        }
    }
    Ok(cursor)
}

/// A [`DecisionReplaySink`] backed by a tokio mpsc sender so a downstream
/// simulator can subscribe as a reactive stream with backpressure (a bounded
/// channel blocks the replay when the consumer is slow).
pub struct ChannelReplaySink {
    tx: tokio::sync::mpsc::Sender<(StepRef, ToolCallRecord)>,
}

impl ChannelReplaySink {
    /// Build a sink + its receiver. `buffer` bounds in-flight decisions
    /// (backpressure). The receiver is the simulator's subscription.
    pub fn new(buffer: usize) -> (Self, tokio::sync::mpsc::Receiver<(StepRef, ToolCallRecord)>) {
        let (tx, rx) = tokio::sync::mpsc::channel(buffer.max(1));
        (Self { tx }, rx)
    }
}

#[async_trait]
impl DecisionReplaySink for ChannelReplaySink {
    async fn emit(&self, step: StepRef, decision: &ToolCallRecord) -> Result<()> {
        self.tx
            .send((step, decision.clone()))
            .await
            .map_err(|e| atomr_agents_core::AgentError::Internal(format!("replay channel closed: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_core::{RunId, Value, WorkflowId};
    use atomr_agents_state::{
        CheckpointKey, InMemoryStepRecordStore, StepRecord, StepRecordStore, ToolCallRecord,
    };
    use parking_lot::Mutex;
    use serde_json::json;
    use std::sync::Arc;

    fn key(step: u64) -> CheckpointKey {
        CheckpointKey {
            workflow_id: WorkflowId::from("wf"),
            run_id: RunId::from("r"),
            super_step: step,
        }
    }

    fn tc(tool: &str, side_effecting: bool, ret: Value) -> ToolCallRecord {
        ToolCallRecord {
            tool: tool.into(),
            args_hash: "h".into(),
            ret,
            is_side_effecting: side_effecting,
        }
    }

    #[derive(Default)]
    struct CollectSink {
        seen: Mutex<Vec<(u64, String)>>,
    }
    #[async_trait]
    impl DecisionReplaySink for CollectSink {
        async fn emit(&self, step: StepRef, decision: &ToolCallRecord) -> Result<()> {
            self.seen.lock().push((step.super_step, decision.tool.clone()));
            Ok(())
        }
    }

    async fn store_with_records() -> InMemoryStepRecordStore {
        let store = InMemoryStepRecordStore::new();
        // step 0: pure only (skipped)
        store
            .save_record(StepRecord::new(key(0)).with_tool_call(tc("read_quote", false, json!({}))))
            .await
            .unwrap();
        // step 1: one side-effecting order
        store
            .save_record(StepRecord::new(key(1)).with_tool_call(tc("place_order", true, json!({"id": 1}))))
            .await
            .unwrap();
        // step 2: mix; one side-effecting
        store
            .save_record(
                StepRecord::new(key(2))
                    .with_tool_call(tc("read_quote", false, json!({})))
                    .with_tool_call(tc("cancel_order", true, json!({"id": 2}))),
            )
            .await
            .unwrap();
        store
    }

    #[tokio::test]
    async fn replays_side_effecting_in_order_skips_pure() {
        let store = store_with_records().await;
        let sink = CollectSink::default();
        let cursor = replay_to_sink(&store, "wf", "r", 0, &sink).await.unwrap();
        let seen = sink.seen.lock().clone();
        assert_eq!(
            seen,
            vec![(1, "place_order".to_string()), (2, "cancel_order".to_string())]
        );
        assert_eq!(cursor, 2, "cursor is the last emitted super_step");
    }

    #[tokio::test]
    async fn cursor_resume_skips_already_emitted() {
        let store = store_with_records().await;
        let sink = CollectSink::default();
        // Resume after step 1 → only step 2 emits.
        let cursor = replay_to_sink(&store, "wf", "r", 2, &sink).await.unwrap();
        let seen = sink.seen.lock().clone();
        assert_eq!(seen, vec![(2, "cancel_order".to_string())]);
        assert_eq!(cursor, 2);
    }

    #[tokio::test]
    async fn empty_run_returns_starting_cursor() {
        let store = InMemoryStepRecordStore::new();
        let sink = CollectSink::default();
        let cursor = replay_to_sink(&store, "wf", "r", 5, &sink).await.unwrap();
        assert_eq!(cursor, 5);
        assert!(sink.seen.lock().is_empty());
    }

    #[tokio::test]
    async fn channel_sink_streams_to_subscriber() {
        let store = store_with_records().await;
        let (sink, mut rx) = ChannelReplaySink::new(8);
        let store2: Arc<dyn StepRecordStore> = Arc::new(store);
        let handle = tokio::spawn(async move {
            replay_to_sink(store2.as_ref(), "wf", "r", 0, &sink).await.unwrap()
        });
        let mut got = Vec::new();
        while let Some((step, dec)) = rx.recv().await {
            got.push((step.super_step, dec.tool));
        }
        let cursor = handle.await.unwrap();
        assert_eq!(cursor, 2);
        assert_eq!(got, vec![(1, "place_order".to_string()), (2, "cancel_order".to_string())]);
    }
}
