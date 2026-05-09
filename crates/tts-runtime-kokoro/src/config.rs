use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KokoroConfig {
    /// Path to the Kokoro `.onnx` model.
    pub model_path: PathBuf,
    /// Path to the voice-embeddings file (`voices.bin` or per-voice `.bin`).
    pub voices_path: PathBuf,
    /// Default voice ID (one of [`crate::KOKORO_VOICES`]).
    #[serde(default = "default_voice")]
    pub default_voice: String,
    /// Speaking speed multiplier (1.0 = normal).
    #[serde(default = "default_speed")]
    pub speed: f32,
    #[serde(default)]
    pub use_gpu: bool,
}

fn default_voice() -> String { "af_alloy".to_string() }
fn default_speed() -> f32 { 1.0 }
