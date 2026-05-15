//! Spec rows: "`SttConversation` <-> `TurnInput` / `Message` round-trip
//! (last turn = user)" and "`append_agent_reply` keeps a full record".

use std::sync::Arc;

use atomr_agents_core::MessageRole;
use atomr_agents_stt_core::{MockSpeechToText, PcmBuffer};
use atomr_agents_stt_harness::{
    AudioSource, SpeakerMap, StreamEndTermination, StreamingLoop, SttHarness, SttHarnessSpec,
};

#[tokio::test]
async fn run_then_map_to_turn_input() {
    let backend = Arc::new(MockSpeechToText::new().with_text("what is the weather"));
    let h = SttHarness::new(
        SttHarnessSpec::new("map-test"),
        backend,
        AudioSource::Pcm(PcmBuffer::new(vec![0.0; 16_000], 16_000, 1)),
        StreamingLoop::default(),
        StreamEndTermination,
    );
    let conversation = h.run().await.expect("run");

    // The single committed turn becomes the agent's `user` input.
    let turn_input = conversation
        .to_turn_input(&SpeakerMap::default())
        .expect("a turn input");
    assert_eq!(turn_input.user, "what is the weather");
    assert!(turn_input.history.is_empty());
}

#[tokio::test]
async fn multi_turn_conversation_maps_last_to_user_rest_to_history() {
    // Build a conversation by hand (the run path only ever produces a
    // single mock turn) and exercise the agentic mapping end to end.
    let mut conversation = atomr_agents_stt_harness::SttConversation::new("c1");
    for line in ["first", "second", "third"] {
        conversation.commit_segment(atomr_agents_stt_core::Segment {
            text: line.into(),
            start_ms: 0,
            end_ms: 0,
            words: vec![],
            speaker: Some(atomr_agents_stt_core::SpeakerTag { id: 0, label: None }),
            confidence: None,
        });
    }

    let turn_input = conversation
        .to_turn_input(&SpeakerMap::default())
        .expect("a turn input");
    assert_eq!(turn_input.user, "third");
    assert_eq!(turn_input.history.len(), 2);
    assert_eq!(turn_input.history[0].content, "first");

    // Appending the agent's reply keeps the conversation a complete
    // record of the exchange.
    conversation.append_agent_reply("the answer");
    let messages = conversation.to_messages(&SpeakerMap::default());
    assert_eq!(messages.len(), 4);
    assert!(matches!(messages[3].role, MessageRole::Assistant));
    assert_eq!(messages[3].content, "the answer");
}
