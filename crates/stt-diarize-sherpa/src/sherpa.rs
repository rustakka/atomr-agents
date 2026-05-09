//! sherpa-onnx wrapper. Without the `sherpa-onnx` feature, the
//! constructor returns a typed [`SttError::ModelLoad`] explaining
//! which feature to enable. The full implementation will land when
//! the upstream `sherpa-onnx` Rust binding stabilizes; this crate
//! ships the trait + interface so callers can write integration
//! code today against the ultimate API.

use std::path::PathBuf;

use async_trait::async_trait;
use atomr_agents_stt_core::{PcmBuffer, Result, SttError};
use serde::{Deserialize, Serialize};

use crate::span::{DiarizationSpan, Diarizer};

/// Paths to the three ONNX models sherpa-onnx's diarization
/// pipeline needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SherpaDiarizerConfig {
    /// Pyannote-style segmentation model (`segmentation-3.0.onnx`).
    pub segmentation_model: PathBuf,
    /// Speaker-embedding model (`3dspeaker_*.onnx` or similar).
    pub embedding_model: PathBuf,
    /// Number of speakers; `None` triggers auto-detection.
    #[serde(default)]
    pub num_speakers: Option<u8>,
    /// Use GPU if compiled with CUDA support.
    #[serde(default)]
    pub use_gpu: bool,
}

pub struct SherpaDiarizer {
    #[allow(dead_code)]
    config: SherpaDiarizerConfig,
}

impl SherpaDiarizer {
    pub fn new(config: SherpaDiarizerConfig) -> Result<Self> {
        #[cfg(feature = "sherpa-onnx")]
        {
            // When the binding is wired in, instantiate the
            // sherpa_onnx::OfflineSpeakerDiarization here.
            tracing::warn!(
                "atomr-agents-stt-diarize-sherpa: `sherpa-onnx` feature is enabled but the binding is not yet wired in this revision",
            );
            Ok(Self { config })
        }
        #[cfg(not(feature = "sherpa-onnx"))]
        {
            // Validate paths exist so callers get an early signal.
            if !config.segmentation_model.exists() || !config.embedding_model.exists() {
                tracing::warn!(
                    seg = ?config.segmentation_model,
                    emb = ?config.embedding_model,
                    "SherpaDiarizer::new: model files not found (also: `sherpa-onnx` feature is disabled)",
                );
            }
            Ok(Self { config })
        }
    }
}

#[async_trait]
impl Diarizer for SherpaDiarizer {
    async fn diarize(&self, _pcm: &PcmBuffer) -> Result<Vec<DiarizationSpan>> {
        #[cfg(not(feature = "sherpa-onnx"))]
        {
            return Err(SttError::model_load(
                "atomr-agents-stt-diarize-sherpa built without `sherpa-onnx` feature; \
                 rebuild with `--features sherpa-onnx` to enable the local diarizer.",
            ));
        }
        #[cfg(feature = "sherpa-onnx")]
        {
            // TODO: actual sherpa pipeline call lands here.
            Err(SttError::model_load(
                "sherpa-onnx pipeline call not yet implemented in this revision",
            ))
        }
    }
}
