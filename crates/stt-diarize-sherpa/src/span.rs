use async_trait::async_trait;
use atomr_agents_stt_core::{PcmBuffer, Result};
use serde::{Deserialize, Serialize};

/// One contiguous span of audio attributed to a single speaker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiarizationSpan {
    pub start_ms: u32,
    pub end_ms: u32,
    pub speaker_id: u8,
    pub confidence: Option<f32>,
}

#[async_trait]
pub trait Diarizer: Send + Sync + 'static {
    /// Run diarization on a chunk of mono PCM. Spans are sorted by
    /// `start_ms` and never overlap.
    async fn diarize(&self, pcm: &PcmBuffer) -> Result<Vec<DiarizationSpan>>;
}
