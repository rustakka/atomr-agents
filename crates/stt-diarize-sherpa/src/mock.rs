//! Deterministic 2-speaker mock. Splits the input into N-second
//! chunks and round-robins between speakers `0` and `1`.

use async_trait::async_trait;
use atomr_agents_stt_core::{PcmBuffer, Result};

use crate::span::{DiarizationSpan, Diarizer};

pub struct MockDiarizer {
    chunk_secs: f32,
    n_speakers: u8,
}

impl Default for MockDiarizer {
    fn default() -> Self {
        Self {
            chunk_secs: 2.5,
            n_speakers: 2,
        }
    }
}

impl MockDiarizer {
    pub fn new(chunk_secs: f32, n_speakers: u8) -> Self {
        Self {
            chunk_secs,
            n_speakers: n_speakers.max(1),
        }
    }
}

#[async_trait]
impl Diarizer for MockDiarizer {
    async fn diarize(&self, pcm: &PcmBuffer) -> Result<Vec<DiarizationSpan>> {
        let total_secs = pcm.duration_secs();
        if total_secs <= 0.0 {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        let mut start = 0.0f32;
        let mut sid: u8 = 0;
        while start < total_secs {
            let end = (start + self.chunk_secs).min(total_secs);
            out.push(DiarizationSpan {
                start_ms: (start * 1000.0) as u32,
                end_ms: (end * 1000.0) as u32,
                speaker_id: sid,
                confidence: Some(1.0),
            });
            sid = (sid + 1) % self.n_speakers;
            start = end;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_robin_two_speakers() {
        let d = MockDiarizer::new(1.0, 2);
        let pcm = PcmBuffer::new(vec![0.0; 16_000 * 3], 16_000, 1); // 3s
        let spans = d.diarize(&pcm).await.unwrap();
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].speaker_id, 0);
        assert_eq!(spans[1].speaker_id, 1);
        assert_eq!(spans[2].speaker_id, 0);
        assert_eq!(spans[2].end_ms, 3_000);
    }
}
