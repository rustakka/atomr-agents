//! Cloud integration test for AssemblyAI. Run with:
//!
//! ```text
//! ASSEMBLYAI_API_KEY=... cargo test -p atomr-agents-stt-runtime-assemblyai \
//!   --features integration -- --ignored
//! ```

#![cfg(feature = "integration")]

use atomr_agents_stt_core::{AudioInput, SpeechToText, TranscribeOptions};
use atomr_agents_stt_runtime_assemblyai::{AssemblyAiConfig, AssemblyAiRunner};
use std::path::PathBuf;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../stt-core/tests/fixtures/jfk.wav")
}

#[tokio::test]
#[ignore = "requires ASSEMBLYAI_API_KEY + network"]
async fn transcribes_jfk_fixture() {
    let _ = std::env::var("ASSEMBLYAI_API_KEY")
        .expect("set ASSEMBLYAI_API_KEY to run this integration test");
    let path = fixture_path();
    if !path.exists() {
        eprintln!("missing fixture {path:?} — copy jfk.wav from whisper.cpp");
        return;
    }
    let runner = AssemblyAiRunner::new(AssemblyAiConfig::from_env()).unwrap();
    let t = runner
        .transcribe(AudioInput::File(path), TranscribeOptions::default())
        .await
        .expect("assemblyai transcribe");
    assert!(!t.text.is_empty(), "got empty transcript");
    assert!(
        t.text.to_lowercase().contains("country"),
        "expected 'country' in transcript: {}",
        t.text
    );
}
