//! Minimal WAV writer used by tests and by cloud backends that
//! need to serialize a [`PcmBuffer`] back into something they can
//! upload (multipart `audio/wav`).

use std::io::Cursor;

use atomr_agents_stt_core::{PcmBuffer, SttError};
use bytes::Bytes;
use hound::{SampleFormat, WavSpec, WavWriter};

pub fn pcm_to_wav_bytes(pcm: &PcmBuffer) -> Result<Bytes, SttError> {
    let spec = WavSpec {
        channels: pcm.channels,
        sample_rate: pcm.sample_rate,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut buf: Vec<u8> = Vec::with_capacity(44 + pcm.samples.len() * 2);
    {
        let cursor = Cursor::new(&mut buf);
        let mut w =
            WavWriter::new(cursor, spec).map_err(|e| SttError::internal(format!("wav writer: {e}")))?;
        for s in &pcm.samples {
            // Saturating cast f32 [-1.0, 1.0] → i16.
            let clamped = s.clamp(-1.0, 1.0);
            let q = (clamped * i16::MAX as f32) as i16;
            w.write_sample(q)
                .map_err(|e| SttError::internal(format!("wav write_sample: {e}")))?;
        }
        w.finalize()
            .map_err(|e| SttError::internal(format!("wav finalize: {e}")))?;
    }
    Ok(Bytes::from(buf))
}
