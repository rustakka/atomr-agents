//! End-to-end: an `SttHarness` over `MockSpeechToText` + an in-memory
//! PCM source. Spec rows: "harness drives STT as a loop, emits
//! `HarnessIteration`" and "partials fold, finals commit ordered
//! turns".

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use atomr_agents_core::{Event, EventEnvelope};
use atomr_agents_stt_core::{MockSpeechToText, PcmBuffer};
use atomr_agents_stt_harness::{
    AudioSource, StreamEndTermination, StreamingLoop, SttHarness, SttHarnessEvent, SttHarnessSpec,
};
use atomr_agents_stt_voice::VoiceMode;
use parking_lot::Mutex;

/// One second of 16 kHz mono silence — the deterministic CI audio path.
fn one_second_pcm() -> AudioSource {
    AudioSource::Pcm(PcmBuffer::new(vec![0.0; 16_000], 16_000, 1))
}

fn harness(voice_mode: VoiceMode) -> SttHarness<StreamingLoop, StreamEndTermination> {
    let backend = Arc::new(MockSpeechToText::new().with_text("hello world"));
    let spec = SttHarnessSpec::new("pipeline-test").with_voice_mode(voice_mode);
    SttHarness::new(
        spec,
        backend,
        one_second_pcm(),
        StreamingLoop::new(voice_mode),
        StreamEndTermination,
    )
}

#[tokio::test]
async fn partials_then_finals_commit_one_ordered_turn() {
    let h = harness(VoiceMode::default());
    let conversation = h.run().await.expect("run");

    // MockStreamingSession emits a partial per push then one Final on
    // finish — so exactly one committed turn, carrying the fixed text.
    assert_eq!(conversation.turns.len(), 1);
    assert_eq!(conversation.turns[0].index, 0);
    assert_eq!(conversation.turns[0].text, "hello world");
    assert!(conversation.open_partial.is_none());
    assert_eq!(conversation.backend.as_ref().map(|b| b.as_str()), Some("mock"));
}

#[tokio::test]
async fn emits_harness_iterations_to_the_event_bus() {
    let h = harness(VoiceMode::default());

    let captured: Arc<Mutex<Vec<EventEnvelope>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let captured = captured.clone();
        h.bus.subscribe(move |env| captured.lock().push(env.clone()));
    }

    h.run().await.expect("run");

    let events = captured.lock();
    let outcomes: Vec<String> = events
        .iter()
        .filter_map(|env| match &env.event {
            Event::HarnessIteration { outcome, .. } => Some(outcome.clone()),
            _ => None,
        })
        .collect();

    assert!(
        outcomes.iter().any(|o| o == "stt_open"),
        "expected an stt_open iteration, got {outcomes:?}"
    );
    assert!(
        outcomes.iter().any(|o| o.starts_with("done:")),
        "expected a done:* iteration, got {outcomes:?}"
    );
    // Every emitted event carries a run id (LangSmith-style tracing).
    assert!(events.iter().all(|env| env.run_id.is_some()));
}

#[tokio::test]
async fn emits_domain_events_on_the_subscriber_stream() {
    let h = harness(VoiceMode::Live);
    // Subscribe before running so nothing is missed.
    let mut stream = h.events();

    h.run().await.expect("run");

    // After `run()` returns, every event is buffered; drain until the
    // terminal `Finished`.
    let mut kinds: Vec<&'static str> = Vec::new();
    let partials = AtomicUsize::new(0);
    while let Some(ev) = stream.recv().await {
        let terminal = matches!(ev, SttHarnessEvent::Finished { .. });
        match ev {
            SttHarnessEvent::Started { .. } => kinds.push("started"),
            SttHarnessEvent::Partial { .. } => {
                partials.fetch_add(1, Ordering::Relaxed);
            }
            SttHarnessEvent::UtteranceCommitted { .. } => kinds.push("committed"),
            SttHarnessEvent::Finished { turn_count, .. } => {
                assert_eq!(turn_count, 1);
                kinds.push("finished");
            }
            _ => {}
        }
        if terminal {
            break;
        }
    }

    assert!(kinds.contains(&"started"), "kinds={kinds:?}");
    assert!(kinds.contains(&"committed"), "kinds={kinds:?}");
    assert!(kinds.contains(&"finished"), "kinds={kinds:?}");
    // Live mode surfaces partials as events.
    assert!(partials.load(Ordering::Relaxed) > 0, "expected live partials");
}
