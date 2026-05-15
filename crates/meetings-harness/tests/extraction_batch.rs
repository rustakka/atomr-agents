//! End-to-end test: batch extraction over an in-memory transcript.

use std::sync::Arc;

use atomr_agents_meetings_harness::{
    AnalysisState, BatchExtractionLoop, InMemoryMeetingsStore, IterationCapTermination,
    MeetingsHarness, MeetingsHarnessSpec, MeetingsStore, RuleBasedExtractor, RunMode,
};
use atomr_agents_stt_core::{Segment, SpeakerTag};
use atomr_agents_stt_harness::{ConversationStore, InMemoryConversationStore, SttConversation};

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

fn sample_conversation() -> SttConversation {
    let mut c = SttConversation::new("call-batch-1");
    c.commit_segment(segment("Hi everyone, glad you could join.", 0, 0, 2_000));
    c.commit_segment(segment("Thanks for hosting, Alice.", 1, 2_000, 4_000));
    c.commit_segment(segment(
        "Let's start with the Q3 roadmap. I'll send a draft tomorrow.",
        0,
        4_000,
        9_000,
    ));
    c.commit_segment(segment("We need to ship the auth rewrite by Friday.", 1, 9_000, 13_000));
    c.commit_segment(segment("Sounds good.", 0, 13_000, 14_000));
    c
}

#[tokio::test]
async fn batch_extraction_produces_attendees_notes_actions_and_tldr() {
    let transcripts: Arc<dyn ConversationStore> = Arc::new(InMemoryConversationStore::new());
    transcripts.put(&sample_conversation()).await.unwrap();

    let analysis_store = Arc::new(InMemoryMeetingsStore::new());

    let spec = MeetingsHarnessSpec::new("meetings", "claude-opus-4-7").with_mode(RunMode::Batch);
    let extractor = Arc::new(RuleBasedExtractor::new());
    let harness = MeetingsHarness::new(
        spec,
        transcripts.clone(),
        analysis_store.clone(),
        extractor,
        BatchExtractionLoop,
        IterationCapTermination::new(8),
    );

    let analysis = harness.run("call-batch-1").await.expect("run ok");

    // Identity:
    assert_eq!(analysis.id, "call-batch-1");
    assert_eq!(analysis.source_transcript_id, "call-batch-1");
    assert_eq!(analysis.state, AnalysisState::Final);
    assert_eq!(analysis.model_id.as_deref(), Some("claude-opus-4-7"));

    // Attendees: two distinct speakers (0, 1).
    assert_eq!(analysis.attendees.len(), 2);
    let tags: Vec<u8> = analysis
        .attendees
        .iter()
        .flat_map(|a| a.speaker_tags.iter().copied())
        .collect();
    assert!(tags.contains(&0) && tags.contains(&1));

    // Notes: one per non-empty turn (5 turns).
    assert_eq!(analysis.notes.len(), 5);

    // Actions: the regex catches "I'll send" and "we need to ship". Both
    // get owners attributed to the speaker who said them.
    assert!(
        analysis.actions.len() >= 2,
        "expected at least 2 actions, got {}: {:#?}",
        analysis.actions.len(),
        analysis.actions
    );
    for action in &analysis.actions {
        assert!(action.owner_attendee_id.is_some(), "every action needs an owner");
    }

    // Summaries: at least one finalized segment plus a TL;DR.
    assert!(!analysis.summary_levels.segments.is_empty());
    assert!(analysis.summary_levels.segments.iter().all(|s| s.finalized));
    assert!(analysis.summary_levels.tldr.is_some());
    assert!(analysis.summary_levels.running.is_some());

    // Persisted under same id as transcript.
    let reloaded = analysis_store.get("call-batch-1").await.unwrap().unwrap();
    assert_eq!(reloaded.notes.len(), analysis.notes.len());
    assert_eq!(reloaded.actions.len(), analysis.actions.len());
    assert_eq!(reloaded.state, AnalysisState::Final);
}

#[tokio::test]
async fn missing_transcript_yields_error() {
    let transcripts: Arc<dyn ConversationStore> = Arc::new(InMemoryConversationStore::new());
    let analysis_store = Arc::new(InMemoryMeetingsStore::new());
    let spec = MeetingsHarnessSpec::new("meetings", "claude-opus-4-7");
    let extractor = Arc::new(RuleBasedExtractor::new());
    let harness = MeetingsHarness::new(
        spec,
        transcripts,
        analysis_store,
        extractor,
        BatchExtractionLoop,
        IterationCapTermination::new(4),
    );
    let err = harness.run("nope").await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("source transcript not found"), "got: {msg}");
}
