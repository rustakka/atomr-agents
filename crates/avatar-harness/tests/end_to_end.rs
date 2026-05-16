//! End-to-end smoke test for [`AvatarHarness`].
//!
//! Uses the in-crate `CapturingSink`, the workspace's
//! [`MockTextToSpeech`], and a stub [`AvatarInferenceClient`] to
//! drive a full perception → cognition → synthesis → sync → sink
//! pipeline with zero network or model dependencies.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use atomr_agents_avatar_core::AvatarSink;
use atomr_agents_avatar_harness::test_support::CapturingSink;
use atomr_agents_avatar_harness::{
    AvatarHarnessBuilder, AvatarHarnessConfig, AvatarInferenceClient, SyncConfig,
};
use atomr_agents_tts_core::{MockTextToSpeech, VoiceRef};
use atomr_infer_core::batch::ExecuteBatch;

struct ScriptedClient {
    responses: Mutex<Vec<String>>,
}

#[async_trait]
impl AvatarInferenceClient for ScriptedClient {
    async fn complete(
        &self,
        _batch: ExecuteBatch,
    ) -> atomr_agents_avatar_core::Result<String> {
        Ok(self.responses.lock().await.remove(0))
    }
}

#[tokio::test]
async fn pipeline_emits_frames_through_capturing_sink() {
    let inference = Arc::new(ScriptedClient {
        responses: Mutex::new(vec![
            r#"{"response_text":"Hello, friend!","emotion_delta":{"valence":0.5,"arousal":0.3,"anger":0.0,"surprise":0.0,"tension":0.0},"gesture":"wave"}"#.to_string(),
        ]),
    });
    let tts = Arc::new(MockTextToSpeech::new());

    let harness = AvatarHarnessBuilder::new()
        .with_inference(inference)
        .with_tts(tts, VoiceRef::library("mock"))
        .with_config(AvatarHarnessConfig {
            sync: SyncConfig {
                frame_rate: 30,
                apply_emotion: true,
            },
            ..Default::default()
        })
        .build()
        .expect("build");

    let sink = Arc::new(CapturingSink::new());
    let frames_handle = sink.handle();
    harness
        .attach_sink(sink.clone() as Arc<dyn AvatarSink>)
        .await
        .expect("attach");

    harness.user_said("hi").await.expect("user_said");

    // Wait until at least one frame lands.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let n = frames_handle.lock().await.len();
        if n > 0 {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!("no frames emitted after 5s");
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    // Verify intent was parsed.
    let intent = harness.last_intent().await.expect("intent recorded");
    assert_eq!(intent.response_text, "Hello, friend!");

    // Emotion state should have moved positive.
    let emo = harness.emotion();
    assert!(emo.valence > 0.0);

    harness.shutdown().await.expect("shutdown");

    let frames = frames_handle.lock().await;
    assert!(!frames.is_empty(), "expected at least one frame");
    // Frame[0] should have weights and a timecode at frame 0.
    assert_eq!(frames[0].timecode.frames, 0);
}

#[tokio::test]
async fn speak_text_bypasses_cognition() {
    let inference = Arc::new(ScriptedClient {
        responses: Mutex::new(Vec::new()), // never called
    });
    let tts = Arc::new(MockTextToSpeech::new());

    let harness = AvatarHarnessBuilder::new()
        .with_inference(inference)
        .with_tts(tts, VoiceRef::library("mock"))
        .build()
        .expect("build");

    let sink = Arc::new(CapturingSink::new());
    let frames_handle = sink.handle();
    harness
        .attach_sink(sink.clone() as Arc<dyn AvatarSink>)
        .await
        .expect("attach");

    harness.speak_text("hello world").await.expect("speak");

    // speak_text is synchronous w.r.t. the harness — frames may still
    // be in flight to the sink task. Give it a brief moment.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(!frames_handle.lock().await.is_empty());

    harness.shutdown().await.expect("shutdown");
}
