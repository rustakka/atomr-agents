//! Spec row: "`SttHarnessRef` is `Callable`, composes in a `Pipeline`".
//!
//! A real `AgentRef` needs an inference client and the full agent
//! pipeline; here a labelled `FnCallable` stands in for the agent. The
//! point under test is composition: the STT harness produces a
//! conversation JSON value that flows into the next stage.

use std::sync::Arc;
use std::time::Duration;

use atomr_agents_callable::{Callable, CallableHandle, FnCallable, Pipeline};
use atomr_agents_core::{CallCtx, IterationBudget, MoneyBudget, TimeBudget, TokenBudget, Value};
use atomr_agents_stt_core::{MockSpeechToText, PcmBuffer};
use atomr_agents_stt_harness::{AudioSource, StreamEndTermination, StreamingLoop, SttHarnessSpec};

fn ctx() -> CallCtx {
    CallCtx {
        agent_id: None,
        tokens: TokenBudget::new(1000),
        time: TimeBudget::new(Duration::from_secs(10)),
        money: MoneyBudget::from_usd(1.0),
        iterations: IterationBudget::new(10),
        trace: vec![],
    }
}

fn harness_ref() -> atomr_agents_stt_harness::SttHarnessRef {
    let backend = Arc::new(MockSpeechToText::new().with_text("composed input"));
    SttHarnessSpec::new("callable-test").into_harness(
        backend,
        AudioSource::Pcm(PcmBuffer::new(vec![0.0; 16_000], 16_000, 1)),
        Box::new(StreamingLoop::default()),
        Box::new(StreamEndTermination),
    )
}

#[tokio::test]
async fn stt_harness_ref_is_callable() {
    let href = harness_ref();
    let out = href.call(Value::Null, ctx()).await.expect("call");
    // The harness returns the conversation as a JSON value.
    let turns = out["turns"].as_array().expect("turns array");
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0]["text"], "composed input");
}

#[tokio::test]
async fn stt_harness_composes_in_a_pipeline() {
    // Stage 2 stands in for an agent: it pulls the last turn's text out
    // of the conversation JSON, exactly as `to_turn_input` would.
    let agent_stub: CallableHandle = Arc::new(FnCallable::labeled(
        "agent-stub",
        |conv: Value, _ctx| async move {
            let user = conv["turns"]
                .as_array()
                .and_then(|t| t.last())
                .and_then(|t| t["text"].as_str())
                .unwrap_or_default()
                .to_string();
            Ok(Value::String(format!("agent saw: {user}")))
        },
    ));

    let href: CallableHandle = Arc::new(harness_ref());
    let pipeline = Pipeline::from(href).then(agent_stub).build();

    let out = pipeline.call(Value::Null, ctx()).await.expect("pipeline");
    assert_eq!(out, Value::String("agent saw: composed input".into()));
}
