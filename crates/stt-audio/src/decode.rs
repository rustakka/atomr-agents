//! Symphonia-backed decoder. Turns any `AudioInput` (file or bytes)
//! into a single [`PcmBuffer`] of interleaved f32 samples in the
//! source's native sample-rate / channel layout. Resampling /
//! channel mixing is the caller's responsibility (use
//! [`crate::resample`]).

use std::io::Cursor;

use atomr_agents_stt_core::{AudioInput, PcmBuffer, SttError};
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Decode any supported container into PCM (f32 interleaved). Runs
/// on the current thread; for large files dispatch via
/// `tokio::task::spawn_blocking`.
pub fn decode_to_pcm(input: AudioInput) -> Result<PcmBuffer, SttError> {
    let (bytes, hint_ext) = read_input(input)?;
    let cursor = Cursor::new(bytes);
    let mss = MediaSourceStream::new(Box::new(cursor), MediaSourceStreamOptions::default());

    let mut hint = Hint::new();
    if let Some(ext) = hint_ext {
        hint.with_extension(&ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| SttError::decode(format!("symphonia probe: {e}")))?;
    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| SttError::decode("no decodable track"))?;
    let track_id = track.id;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| SttError::decode("track missing sample rate"))?;
    let channels = track
        .codec_params
        .channels
        .ok_or_else(|| SttError::decode("track missing channel layout"))?
        .count() as u16;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| SttError::decode(format!("codec: {e}")))?;

    let mut samples: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphError::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphError::ResetRequired) => break,
            Err(e) => return Err(SttError::decode(format!("packet: {e}"))),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = decoder
            .decode(&packet)
            .map_err(|e| SttError::decode(format!("decode: {e}")))?;
        append_interleaved(&decoded, &mut samples);
    }

    Ok(PcmBuffer::new(samples, sample_rate, channels))
}

fn append_interleaved(buf: &AudioBufferRef<'_>, out: &mut Vec<f32>) {
    use symphonia::core::audio::AudioBuffer;
    use symphonia::core::sample::Sample;
    fn extend<S: Sample + symphonia::core::conv::IntoSample<f32> + Copy>(
        buf: &AudioBuffer<S>,
        out: &mut Vec<f32>,
    ) {
        let frames = buf.frames();
        let chs = buf.spec().channels.count();
        for f in 0..frames {
            for c in 0..chs {
                let s = buf.chan(c)[f];
                out.push(s.into_sample());
            }
        }
    }
    match buf {
        AudioBufferRef::U8(b) => extend(b.as_ref(), out),
        AudioBufferRef::U16(b) => extend(b.as_ref(), out),
        AudioBufferRef::U24(b) => extend(b.as_ref(), out),
        AudioBufferRef::U32(b) => extend(b.as_ref(), out),
        AudioBufferRef::S8(b) => extend(b.as_ref(), out),
        AudioBufferRef::S16(b) => extend(b.as_ref(), out),
        AudioBufferRef::S24(b) => extend(b.as_ref(), out),
        AudioBufferRef::S32(b) => extend(b.as_ref(), out),
        AudioBufferRef::F32(b) => extend(b.as_ref(), out),
        AudioBufferRef::F64(b) => extend(b.as_ref(), out),
    }
}

fn read_input(input: AudioInput) -> Result<(Vec<u8>, Option<String>), SttError> {
    match input {
        AudioInput::File(p) => {
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_string());
            let data = std::fs::read(&p)?;
            Ok((data, ext))
        }
        AudioInput::Bytes { data, format } => {
            Ok((data.to_vec(), Some(format.extension().to_string())))
        }
        AudioInput::Pcm(_) => Err(SttError::decode(
            "decode_to_pcm called with already-decoded PCM input",
        )),
    }
}

/// Mix down to mono by averaging channels.
pub fn to_mono(pcm: &PcmBuffer) -> PcmBuffer {
    if pcm.channels <= 1 {
        return pcm.clone();
    }
    let chs = pcm.channels as usize;
    let frames = pcm.samples.len() / chs;
    let mut mono = Vec::with_capacity(frames);
    for f in 0..frames {
        let mut acc = 0.0f32;
        for c in 0..chs {
            acc += pcm.samples[f * chs + c];
        }
        mono.push(acc / chs as f32);
    }
    PcmBuffer::new(mono, pcm.sample_rate, 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_stt_core::{AudioFormat, AudioInput};
    use bytes::Bytes;

    #[test]
    fn wav_round_trip_decodes_to_correct_sample_rate() {
        // Synthesize a 0.1-second 440 Hz tone at 16 kHz mono and
        // round-trip it through wav writer + symphonia decoder.
        let sr = 16_000u32;
        let frames = sr / 10;
        let samples: Vec<f32> = (0..frames)
            .map(|i| (i as f32 / sr as f32 * 440.0 * std::f32::consts::TAU).sin() * 0.5)
            .collect();
        let pcm = PcmBuffer::new(samples, sr, 1);
        let wav = crate::wav::pcm_to_wav_bytes(&pcm).unwrap();
        let decoded = decode_to_pcm(AudioInput::Bytes {
            data: Bytes::from(wav.to_vec()),
            format: AudioFormat::Wav,
        })
        .unwrap();
        assert_eq!(decoded.sample_rate, sr);
        assert_eq!(decoded.channels, 1);
        // Expect roughly the same frame count (within a frame of
        // header/quantization).
        assert!(
            (decoded.samples.len() as i64 - frames as i64).abs() <= 4,
            "frame count drift: got {}, want {}",
            decoded.samples.len(),
            frames
        );
    }

    #[test]
    fn to_mono_averages_stereo() {
        // 4 frames stereo: [L0,R0, L1,R1, L2,R2, L3,R3].
        let pcm = PcmBuffer::new(
            vec![1.0, 0.0, 0.5, 0.5, -1.0, 1.0, 0.0, 0.0],
            44_100,
            2,
        );
        let mono = to_mono(&pcm);
        assert_eq!(mono.samples, vec![0.5, 0.5, 0.0, 0.0]);
        assert_eq!(mono.channels, 1);
    }
}
