//! whisper.cpp deployment config.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhisperConfig {
    /// Path to a ggml/gguf whisper model file. The `download-models`
    /// feature exposes a helper that fetches a known weight into
    /// `dirs::cache_dir()`.
    pub model_path: PathBuf,
    /// Threads for whisper.cpp's CPU backend.
    #[serde(default = "default_threads")]
    pub n_threads: u16,
    /// Try to use GPU acceleration (CUDA / Metal / CoreML, depending
    /// on which feature this crate was built with).
    #[serde(default)]
    pub gpu: bool,
    /// Default BCP-47 language hint. `None` triggers detection.
    #[serde(default)]
    pub default_language: Option<String>,
    /// Beam size for decoding. `1` = greedy.
    #[serde(default = "default_beam")]
    pub beam_size: u16,
}

fn default_threads() -> u16 {
    std::thread::available_parallelism()
        .map(|n| n.get() as u16)
        .unwrap_or(4)
}

fn default_beam() -> u16 {
    1
}

impl WhisperConfig {
    pub fn new(model_path: impl Into<PathBuf>) -> Self {
        Self {
            model_path: model_path.into(),
            n_threads: default_threads(),
            gpu: false,
            default_language: None,
            beam_size: default_beam(),
        }
    }
}

/// Well-known ggml model identifiers. Used by the optional
/// `download-models` helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WhisperModel {
    TinyEn,
    BaseEn,
    Base,
    SmallEn,
    Small,
    MediumEn,
    Medium,
    LargeV3,
    LargeV3Turbo,
}

impl WhisperModel {
    pub fn ggml_filename(&self) -> &'static str {
        match self {
            WhisperModel::TinyEn => "ggml-tiny.en.bin",
            WhisperModel::BaseEn => "ggml-base.en.bin",
            WhisperModel::Base => "ggml-base.bin",
            WhisperModel::SmallEn => "ggml-small.en.bin",
            WhisperModel::Small => "ggml-small.bin",
            WhisperModel::MediumEn => "ggml-medium.en.bin",
            WhisperModel::Medium => "ggml-medium.bin",
            WhisperModel::LargeV3 => "ggml-large-v3.bin",
            WhisperModel::LargeV3Turbo => "ggml-large-v3-turbo.bin",
        }
    }
}
