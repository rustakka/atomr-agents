//! Spec rows: diarization `Off` / `Backend` / `Layered` behaviour
//! end-to-end, and "caps mismatch emits `DiarizationWarning`, run
//! continues".

use std::sync::Arc;

use atomr_agents_stt_core::{MockSpeechToText, PcmBuffer};
use atomr_agents_stt_diarize_sherpa::MockDiarizer;
use atomr_agents_stt_harness::{
    AudioSource, DiarizationPolicy, StreamEndTermination, StreamingLoop, SttHarness, SttHarnessEvent,
    SttHarnessSpec,
};

fn pcm(secs: usize) -> AudioSource {
    AudioSource::Pcm(PcmBuffer::new(vec![0.0; 16_000 * secs], 16_000, 1))
}

fn run_with(policy: DiarizationPolicy) -> SttHarness<StreamingLoop, StreamEndTermination> {
    let backend = Arc::new(MockSpeechToText::new().with_text("diarized line"));
    SttHarness::new(
        SttHarnessSpec::new("diarize-test").with_diarization(policy),
        backend,
        pcm(2),
        StreamingLoop::default(),
        StreamEndTermination,
    )
}

#[tokio::test]
async fn off_policy_leaves_turns_unattributed() {
    let conversation = run_with(DiarizationPolicy::Off).run().await.expect("run");
    assert_eq!(conversation.turns.len(), 1);
    assert!(conversation.turns[0].speaker_id().is_none());
}

#[tokio::test]
async fn backend_policy_trusts_the_backend() {
    // The mock backend emits a `Final` with no speaker, so `Backend`
    // policy leaves the turn unattributed — it does not invent one.
    let conversation = run_with(DiarizationPolicy::Backend).run().await.expect("run");
    assert_eq!(conversation.turns.len(), 1);
    assert!(conversation.turns[0].speaker_id().is_none());
}

#[tokio::test]
async fn layered_policy_attaches_a_speaker() {
    let policy = DiarizationPolicy::Layered(Arc::new(MockDiarizer::new(1.0, 2)));
    let conversation = run_with(policy).run().await.expect("run");
    assert_eq!(conversation.turns.len(), 1);
    // The layered MockDiarizer round-robins speakers 0/1 over the 2 s
    // utterance; the max-overlap tie resolves to speaker 0.
    assert_eq!(conversation.turns[0].speaker_id(), Some(0));
}

#[tokio::test]
async fn caps_mismatch_warns_but_run_continues() {
    // MockSpeechToText advertises `DiarizationSupport::SpeakerCount`,
    // so a `Layered` policy is redundant — the harness warns.
    let policy = DiarizationPolicy::Layered(Arc::new(MockDiarizer::default()));
    let h = run_with(policy);
    let mut stream = h.events();

    let conversation = h.run().await.expect("run should still succeed");
    assert_eq!(conversation.turns.len(), 1);

    let mut warned = false;
    while let Some(ev) = stream.recv().await {
        let terminal = matches!(ev, SttHarnessEvent::Finished { .. });
        if matches!(ev, SttHarnessEvent::DiarizationWarning { .. }) {
            warned = true;
        }
        if terminal {
            break;
        }
    }
    assert!(warned, "expected a DiarizationWarning event");
}
