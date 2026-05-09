//! Local whisper.cpp integration test. Runs only when both:
//!
//! - `whisper-cpp` Cargo feature is enabled, and
//! - `STT_WHISPER_MODEL` env var points to a ggml/gguf weights file.
//!
//! ```text
//! STT_WHISPER_MODEL=~/models/ggml-base.en.bin \
//!   cargo test -p atomr-agents-stt-runtime-whisper --features whisper-cpp \
//!   -- --ignored
//! ```

#![cfg(feature = "whisper-cpp")]

use atomr_agents_stt_core::{AudioInput, SpeechToText, TranscribeOptions};
use atomr_agents_stt_runtime_whisper::{WhisperConfig, WhisperRunner};
use std::path::PathBuf;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../stt-core/tests/fixtures/jfk.wav")
}

#[tokio::test]
#[ignore = "requires STT_WHISPER_MODEL env + jfk.wav fixture"]
async fn transcribes_jfk_locally() {
    let model = match std::env::var("STT_WHISPER_MODEL") {
        Ok(p) => PathBuf::from(p),
        Err(_) => return,
    };
    let path = fixture_path();
    if !path.exists() {
        eprintln!("missing fixture {path:?}");
        return;
    }

    let runner = WhisperRunner::new(WhisperConfig::new(model)).expect("whisper init");
    let t = runner
        .transcribe(AudioInput::File(path), TranscribeOptions::default())
        .await
        .expect("whisper transcribe");
    assert!(!t.text.is_empty(), "got empty transcript");
    assert!(
        t.text.to_lowercase().contains("country"),
        "expected 'country' in transcript: {}",
        t.text
    );
}
