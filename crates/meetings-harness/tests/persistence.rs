//! Cross-harness persistence test: a single Checkpointer holds both a
//! `SttConversation` (workflow_id = "stt-harness") and a
//! `MeetingAnalysis` (workflow_id = "meetings-harness") under the same
//! `run_id`, i.e. the same `conversation_id`.

#![cfg(feature = "state")]

use std::sync::Arc;

use atomr_agents_meetings_harness::{
    BatchExtractionLoop, CheckpointerMeetingsStore, IterationCapTermination, MeetingsHarness,
    MeetingsHarnessSpec, MeetingsStore, RuleBasedExtractor, RunMode,
};
use atomr_agents_state::{Checkpointer, InMemoryCheckpointer};
use atomr_agents_stt_core::{Segment, SpeakerTag};
use atomr_agents_stt_harness::{CheckpointerConversationStore, ConversationStore, SttConversation};

fn segment(text: &str, speaker: u8) -> Segment {
    Segment {
        text: text.into(),
        start_ms: 0,
        end_ms: 1_000,
        words: vec![],
        speaker: Some(SpeakerTag {
            id: speaker,
            label: None,
        }),
        confidence: Some(1.0),
    }
}

#[tokio::test]
async fn analysis_and_transcript_share_a_conversation_id_in_one_checkpointer() {
    let cp: Arc<dyn Checkpointer> = Arc::new(InMemoryCheckpointer::new());

    let transcript_store: Arc<dyn ConversationStore> =
        Arc::new(CheckpointerConversationStore::new(cp.clone()));
    let analysis_store = Arc::new(CheckpointerMeetingsStore::new(cp.clone()));

    let mut conv = SttConversation::new("session-99");
    conv.commit_segment(segment("Welcome to the meeting.", 0));
    conv.commit_segment(segment("Thanks. I'll prepare the slides.", 1));
    transcript_store.put(&conv).await.unwrap();

    let spec = MeetingsHarnessSpec::new("meetings", "claude-opus-4-7").with_mode(RunMode::Batch);
    let extractor = Arc::new(RuleBasedExtractor::new());
    let harness = MeetingsHarness::new(
        spec,
        transcript_store.clone(),
        analysis_store.clone(),
        extractor,
        BatchExtractionLoop,
        IterationCapTermination::new(8),
    );

    let analysis = harness.run("session-99").await.unwrap();
    assert_eq!(analysis.id, "session-99");
    assert_eq!(analysis.source_transcript_id, "session-99");

    // Reopen both stores over the same checkpointer (simulating a
    // restart) and confirm both records are joined under the same id.
    let transcripts_again = CheckpointerConversationStore::new(cp.clone());
    let analyses_again = CheckpointerMeetingsStore::new(cp);

    let conv2 = transcripts_again.get("session-99").await.unwrap().unwrap();
    let analysis2 = analyses_again.get("session-99").await.unwrap().unwrap();

    assert_eq!(conv2.id, analysis2.source_transcript_id);
    assert_eq!(conv2.turns.len(), 2);
    assert!(!analysis2.notes.is_empty());
    assert!(!analysis2.attendees.is_empty());
}
