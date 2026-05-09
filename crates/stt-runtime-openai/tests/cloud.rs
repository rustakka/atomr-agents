//! Cloud integration test for OpenAI Whisper. Run with:
//!
//! ```text
//! OPENAI_API_KEY=sk-... cargo test -p atomr-agents-stt-runtime-openai \
//!   --features integration -- --ignored
//! ```
//!
//! Requires a small WAV fixture at `crates/stt-core/tests/fixtures/jfk.wav`
//! (the 11-second clip from whisper.cpp's repo).

#![cfg(feature = "integration")]

use atomr_agents_stt_core::{AudioInput, SpeechToText, TranscribeOptions};
use atomr_agents_stt_runtime_openai::{OpenAiSttConfig, OpenAiSttRunner};
use std::path::PathBuf;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../stt-core/tests/fixtures/jfk.wav")
}

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY + network"]
async fn transcribes_jfk_fixture() {
    let _ = std::env::var("OPENAI_API_KEY")
        .expect("set OPENAI_API_KEY to run this integration test");
    let path = fixture_path();
    if !path.exists() {
        eprintln!("missing fixture {path:?} — copy jfk.wav from whisper.cpp");
        return;
    }
    let runner = OpenAiSttRunner::new(OpenAiSttConfig::from_env()).unwrap();
    let t = runner
        .transcribe(AudioInput::File(path), TranscribeOptions::default())
        .await
        .expect("openai transcribe");
    assert!(!t.text.is_empty(), "got empty transcript");
    // The JFK fixture famously contains the word "country".
    assert!(
        t.text.to_lowercase().contains("country"),
        "expected 'country' in transcript: {}",
        t.text
    );
}
