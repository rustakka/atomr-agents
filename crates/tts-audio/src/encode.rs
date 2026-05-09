//! PCM -> container encoders.

use std::io::Cursor;

use atomr_agents_stt_core::{PcmBuffer, SttError};
use bytes::Bytes;
use hound::{SampleFormat, WavSpec, WavWriter};

/// PCM (f32, mono or interleaved) -> WAV (RIFF / PCM-S16LE) bytes.
/// Mirrors `stt-audio::wav::pcm_to_wav_bytes` so callers have one
/// import path for serialising audio output.
pub fn pcm_to_wav_bytes(pcm: &PcmBuffer) -> Result<Bytes, SttError> {
    let spec = WavSpec {
        channels: pcm.channels.max(1),
        sample_rate: pcm.sample_rate.max(1),
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut buf: Vec<u8> = Vec::with_capacity(44 + pcm.samples.len() * 2);
    {
        let cursor = Cursor::new(&mut buf);
        let mut w = WavWriter::new(cursor, spec)
            .map_err(|e| SttError::internal(format!("wav writer: {e}")))?;
        for s in &pcm.samples {
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

/// Concatenate streamed PCM-S16LE bytes into one WAV container.
/// Useful when a backend emits raw PCM frames over a stream and the
/// caller wants to persist them as a single playable file.
pub fn pcm_s16le_chunks_to_wav(
    chunks: &[Bytes],
    sample_rate: u32,
    channels: u16,
) -> Result<Bytes, SttError> {
    let total_samples: usize = chunks.iter().map(|c| c.len() / 2).sum();
    let mut samples = Vec::with_capacity(total_samples);
    for chunk in chunks {
        for pair in chunk.chunks_exact(2) {
            let s = i16::from_le_bytes([pair[0], pair[1]]);
            samples.push(s as f32 / i16::MAX as f32);
        }
    }
    let pcm = PcmBuffer::new(samples, sample_rate, channels);
    pcm_to_wav_bytes(&pcm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_round_trip_via_stt_audio_decoder() {
        // 0.1s of a 440 Hz tone at 16 kHz mono.
        let sr = 16_000u32;
        let frames = sr / 10;
        let samples: Vec<f32> = (0..frames)
            .map(|i| (i as f32 / sr as f32 * 440.0 * std::f32::consts::TAU).sin() * 0.5)
            .collect();
        let pcm = PcmBuffer::new(samples, sr, 1);
        let wav = pcm_to_wav_bytes(&pcm).unwrap();
        // Verify by decoding via the STT-side decoder.
        use atomr_agents_stt_core::{AudioFormat, AudioInput};
        let decoded = atomr_agents_stt_audio::decode::decode_to_pcm(AudioInput::Bytes {
            data: bytes::Bytes::from(wav.to_vec()),
            format: AudioFormat::Wav,
        })
        .unwrap();
        assert_eq!(decoded.sample_rate, sr);
        assert_eq!(decoded.channels, 1);
        // Frame count round-trip stays within a few frames of original.
        assert!((decoded.samples.len() as i64 - frames as i64).abs() <= 4);
    }

    #[test]
    fn pcm_chunks_concatenate_to_wav() {
        let sr = 8_000u32;
        // Two chunks of 16 samples each = 32 PCM-S16LE frames.
        let chunks = vec![
            Bytes::from(vec![0u8; 32]),
            Bytes::from(vec![0u8; 32]),
        ];
        let wav = pcm_s16le_chunks_to_wav(&chunks, sr, 1).unwrap();
        assert!(wav.len() > 44); // header + samples
    }
}
