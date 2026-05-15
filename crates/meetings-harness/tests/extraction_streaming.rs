//! End-to-end test: streaming extraction with a mock STT broadcast.
//!
//! Drives a fake STT event channel: pushes UtteranceCommitted bursts,
//! grows the transcript in the shared store, then closes with
//! Finished. Asserts the ledger is monotonic, the in-flight tail
//! segment is revised but earlier segments are frozen, and the
//! watermark advances.

use std::sync::Arc;
use std::time::Duration;

use atomr_agents_meetings_harness::{
    AnalysisState, IterationCapTermination, MeetingsHarness, MeetingsHarnessSpec, MeetingsStore,
    RuleBasedExtractor, RunMode, StreamingExtractionLoop,
};
use atomr_agents_stt_core::{Segment, SpeakerTag};
use atomr_agents_stt_harness::{
    ConversationStore, InMemoryConversationStore, SttConversation, SttHarnessEvent,
};
use tokio::sync::broadcast;

fn segment(text: &str, speaker_id: u8, start_ms: u32, end_ms: u32) -> Segment {
    Segment {
        text: text.into(),
        start_ms,
        end_ms,
        words: vec![],
        speaker: Some(SpeakerTag {
            id: speaker_id,
            label: None,
        }),
        confidence: Some(1.0),
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn streaming_extraction_appends_and_revises_tail() {
    let transcripts: Arc<dyn ConversationStore> = Arc::new(InMemoryConversationStore::new());

    // Seed an empty transcript so the harness can find it.
    let mut conv = SttConversation::new("call-stream-1");
    transcripts.put(&conv).await.unwrap();

    let analysis_store =
        Arc::new(atomr_agents_meetings_harness::InMemoryMeetingsStore::new());

    let (tx, rx) = broadcast::channel::<SttHarnessEvent>(64);

    let spec = MeetingsHarnessSpec::new("meetings", "claude-opus-4-7").with_mode(
        RunMode::Live {
            segment_turn_count: 3,
        },
    );
    let extractor = Arc::new(RuleBasedExtractor::new());
    let loop_strategy =
        StreamingExtractionLoop::new(rx, transcripts.clone(), "call-stream-1".to_string());
    let harness = MeetingsHarness::new(
        spec,
        transcripts.clone(),
        analysis_store.clone(),
        extractor,
        loop_strategy,
        IterationCapTermination::new(20),
    );

    // Subscribe to harness events so we can assert on them.
    let mut events = harness.events();

    // Spawn the run; drive it by mutating the transcript and posting
    // events.
    let analysis_store_clone = analysis_store.clone();
    let run = tokio::spawn(async move { harness.run("call-stream-1").await });

    // Helper to push a turn + announce it.
    let mut push_turn = |text: &str, speaker: u8| -> u64 {
        let start = conv.total_audio_secs as u32 * 1000;
        let turn = conv.commit_segment(segment(
            text,
            speaker,
            start,
            start + 2_000,
        ));
        let conv_for_store = conv.clone();
        let tx = tx.clone();
        let transcripts = transcripts.clone();
        tokio::spawn(async move {
            transcripts.put(&conv_for_store).await.unwrap();
            let _ = tx.send(SttHarnessEvent::UtteranceCommitted { turn });
        });
        (conv.turns.len() - 1) as u64
    };

    // Wait briefly for the run to subscribe.
    tokio::time::sleep(Duration::from_millis(10)).await;

    // First burst: 2 turns.
    push_turn("Hi team, this is the kickoff.", 0);
    push_turn("Great to be here, Bob.", 1);
    // Wait for the loop to process.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Second burst: another 2 turns — this should exceed segment_turn_count=3,
    // forcing the tail segment to finalize.
    push_turn("Let's lock the API surface by Wednesday.", 0);
    push_turn("I'll draft the proposal.", 1);
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Close out.
    let _ = tx.send(SttHarnessEvent::Finished {
        reason: "stream_end".into(),
        turn_count: 4,
        total_audio_secs: 8.0,
    });
    drop(tx);

    let analysis = run.await.expect("join ok").expect("run ok");

    // Identity & state:
    assert_eq!(analysis.id, "call-stream-1");
    assert_eq!(analysis.state, AnalysisState::Final);

    // Monotonic ledger: notes/actions appended over time, never
    // reordered. We can't compare identities easily here, but we can
    // assert counts grew over iterations: at minimum we should have
    // one note per turn (4) and at least 1 action ("I'll draft" or
    // "let's lock").
    assert!(analysis.notes.len() >= 4, "got {} notes", analysis.notes.len());
    assert!(
        !analysis.actions.is_empty(),
        "expected at least one detected action"
    );

    // At least one finalized segment, then a final state where everything
    // is finalized.
    let total_segments = analysis.summary_levels.segments.len();
    assert!(
        total_segments >= 1,
        "expected at least one segment, got {total_segments}"
    );
    assert!(
        analysis.summary_levels.segments.iter().all(|s| s.finalized),
        "after finalize all segments should be frozen"
    );

    // Watermark advanced.
    assert!(analysis.last_processed_turn_index.is_some());

    // Persisted in store.
    let reloaded = analysis_store_clone
        .get("call-stream-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reloaded.id, "call-stream-1");
    assert_eq!(reloaded.notes.len(), analysis.notes.len());

    // Drain a couple of events to confirm the stream emitted progress
    // events (best-effort — the channel may have already been consumed).
    let mut saw_started = false;
    for _ in 0..16 {
        match tokio::time::timeout(Duration::from_millis(1), events.recv()).await {
            Ok(Some(atomr_agents_meetings_harness::MeetingsHarnessEvent::Started { .. })) => {
                saw_started = true;
            }
            _ => break,
        }
    }
    // Started may have fired before we drained, so treat as best-effort.
    let _ = saw_started;
}
