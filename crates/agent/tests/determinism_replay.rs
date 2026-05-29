//! End-to-end determinism test (FR-1 acceptance).
//!
//! Records a multi-step "live" run into a `RecordingCheckpointer`'s
//! `StepRecordStore`, then replays it through a `ReplayProvider` and
//! asserts:
//! * byte-identical `TurnResult`s in order,
//! * zero provider calls (the `ReplayProvider` has no live provider),
//! * a missing record is a loud error (never silent re-inference),
//! * a `Telemetry` sink observes the replayed steps via `RunEvent`s.

use std::sync::Arc;

use atomr_agents_agent::{record_turn, ReplayProvider, TurnResult};
use atomr_agents_agent::{InferenceClient, Provider};
use atomr_agents_core::{RunId, WorkflowId};
use atomr_agents_observability::{CheckpointRef, InMemoryTelemetrySink, RunEvent, RunEventKind, Telemetry};
use atomr_agents_state::{CheckpointKey, RecordingCheckpointer, StepRecord, StepRecordStore};
use atomr_agents_tool::ParsedToolCall;
use atomr_infer_core::batch::ExecuteBatch;
use atomr_infer_core::tokens::{FinishReason, TokenUsage};

fn batch() -> ExecuteBatch {
    ExecuteBatch {
        request_id: "r".into(),
        model: "claude".into(),
        messages: vec![],
        sampling: Default::default(),
        stream: false,
        estimated_tokens: 0,
    }
}

fn live_turn(text: &str, with_tool: bool) -> TurnResult {
    TurnResult {
        text: text.into(),
        usage: TokenUsage {
            input_tokens: 10,
            output_tokens: 4,
            reasoning_tokens: 0,
            cached_tokens: 1,
        },
        finish_reason: Some(if with_tool {
            FinishReason::ToolCalls
        } else {
            FinishReason::Stop
        }),
        tool_calls: if with_tool {
            vec![ParsedToolCall {
                id: "c1".into(),
                name: "place_order".into(),
                arguments_raw: "{\"qty\":1}".into(),
            }]
        } else {
            vec![]
        },
    }
}

#[tokio::test]
async fn record_then_replay_is_byte_identical_with_zero_provider_calls() {
    let workflow = WorkflowId::from("wf-determinism");
    let run = RunId::from("run-1");

    // --- "Live" run: record each turn into the recording checkpointer. ---
    let recorder = RecordingCheckpointer::in_memory();
    let live_turns = [live_turn("analyze", true), live_turn("final answer", false)];

    for (step, turn) in live_turns.iter().enumerate() {
        let inf = record_turn(
            Provider::Anthropic,
            "claude",
            "1.0",
            "params-abc",
            "prompt-xyz",
            turn,
        );
        recorder
            .record(
                StepRecord::new(CheckpointKey {
                    workflow_id: workflow.clone(),
                    run_id: run.clone(),
                    super_step: step as u64,
                })
                .with_inference(inf),
            )
            .await
            .unwrap();
    }

    // --- Replay: source the model from records, zero provider calls. ---
    let store: Arc<dyn StepRecordStore> = recorder.records();
    let replay = ReplayProvider::load(store.as_ref(), &workflow, &run)
        .await
        .unwrap();

    // Telemetry observes replayed steps.
    let sink = Arc::new(InMemoryTelemetrySink::new());
    let telemetry = Telemetry::new().with_sink(sink.clone());

    for (step, expected) in live_turns.iter().enumerate() {
        let replayed = replay.run(batch()).await.unwrap();
        // Byte-identical reproduction.
        assert_eq!(replayed.text, expected.text);
        assert_eq!(replayed.usage.input_tokens, expected.usage.input_tokens);
        assert_eq!(replayed.usage.cached_tokens, expected.usage.cached_tokens);
        assert_eq!(replayed.finish_reason, expected.finish_reason);
        assert_eq!(replayed.tool_calls, expected.tool_calls);

        telemetry.emit(
            RunEvent::new(RunEventKind::InferenceCompleted).with_checkpoint(CheckpointRef::new(
                workflow.as_str(),
                run.as_str(),
                step as u64,
            )),
        );
    }

    // Exhausted: a further call is a loud, typed error — never re-inference.
    let err = replay.run(batch()).await.unwrap_err();
    assert!(err.to_string().contains("replay"));

    // Every emitted RunEvent resolves to a checkpoint pointer.
    let events = sink.events();
    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|e| e.checkpoint_ref.is_some()));
    assert_eq!(events[1].checkpoint_ref.as_ref().unwrap().super_step, 1);
}
