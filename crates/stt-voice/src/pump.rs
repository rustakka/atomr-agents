//! Glue: read frames from `MicCaptureSession`, pack them as PCM16
//! bytes, and forward to a `StreamingSession::push_audio`.

use atomr_agents_stt_audio::mic::MicCaptureSession;
use atomr_agents_stt_core::{Result, StreamingSession, SttError};
use bytes::Bytes;

/// Run the pump until either the mic closes or `push_audio`
/// returns an error. Caller drives this on its own task.
pub async fn pump_mic_to_stream<S: StreamingSession + ?Sized>(
    mic: &mut MicCaptureSession,
    stream: &mut S,
) -> Result<()> {
    while let Some(frame) = mic.recv().await {
        let bytes = pcm_f32_to_pcm16_le(&frame.samples);
        stream.push_audio(Bytes::from(bytes)).await?;
    }
    stream.finish().await?;
    Ok::<(), SttError>(())
}

fn pcm_f32_to_pcm16_le(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        let q = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        out.extend_from_slice(&q.to_le_bytes());
    }
    out
}
