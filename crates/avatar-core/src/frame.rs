//! Per-tick payload streamed to the avatar sink.
//!
//! An [`AvatarFrame`] is the unit a [`crate::AvatarSink`] consumes:
//! one SMPTE-timecoded slice of audio + a 52-element blendshape
//! vector + optional emotion overlay + optional body-rig hints.

use serde::{Deserialize, Serialize};

use crate::blendshape::BlendshapeWeights;
use crate::emotion::EmotionVector;

/// SMPTE timecode (`HH:MM:SS:FF`) with a frame rate so the receiver
/// can interpret the fourth field unambiguously. Drop-frame is *not*
/// modeled — we run at integer-rate (24 / 30 / 60) for content rates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SmpteTimecode {
    pub hours: u8,
    pub minutes: u8,
    pub seconds: u8,
    pub frames: u8,
    /// Frame rate denominator. Common: 24 / 25 / 30 / 50 / 60.
    pub frame_rate: u8,
}

impl SmpteTimecode {
    /// Build a timecode from an absolute frame counter at a given rate.
    /// Wraps at 24h. Always non-drop-frame.
    pub fn from_frame_index(frame_index: u64, frame_rate: u8) -> Self {
        let fps = frame_rate.max(1) as u64;
        let total_seconds = frame_index / fps;
        let frames = (frame_index % fps) as u8;
        let seconds = (total_seconds % 60) as u8;
        let minutes = ((total_seconds / 60) % 60) as u8;
        let hours = ((total_seconds / 3600) % 24) as u8;
        Self {
            hours,
            minutes,
            seconds,
            frames,
            frame_rate,
        }
    }

    /// Render `HH:MM:SS:FF`.
    pub fn format(&self) -> String {
        format!(
            "{:02}:{:02}:{:02}:{:02}",
            self.hours, self.minutes, self.seconds, self.frames
        )
    }
}

/// PCM audio slice. We standardize on 16-bit signed little-endian
/// because every TTS backend (Piper / OpenAI / ElevenLabs) plus UE5's
/// `USoundWaveProcedural` consume it natively. Higher-fidelity formats
/// can be added without breaking the wire by introducing a new
/// `AudioChunkV2` variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioChunk {
    /// Interleaved signed 16-bit little-endian PCM samples.
    pub samples_s16le: Vec<u8>,
    pub sample_rate_hz: u32,
    pub channels: u8,
}

impl AudioChunk {
    /// Empty 16 kHz mono chunk (the conventional voice-input rate).
    pub fn empty_voice() -> Self {
        Self {
            samples_s16le: Vec::new(),
            sample_rate_hz: 16_000,
            channels: 1,
        }
    }

    /// Duration of this chunk, in seconds.
    pub fn duration_secs(&self) -> f32 {
        if self.sample_rate_hz == 0 || self.channels == 0 {
            return 0.0;
        }
        let bytes_per_sample_per_channel = 2_usize; // s16
        let frames = self.samples_s16le.len()
            / (bytes_per_sample_per_channel * self.channels as usize).max(1);
        frames as f32 / self.sample_rate_hz as f32
    }
}

/// Optional per-frame body-rig hints. UE5 control rig consumes named
/// curves rather than blendshapes, so we ship a free-form map and let
/// the receiver plugin decide which `CTRL_*` names to bind.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BodyRigHint {
    /// Curve name → weight in `[-1.0, 1.0]` (sign matters for sliders
    /// like `pose_BodyLean` that go either direction).
    pub curves: std::collections::BTreeMap<String, f32>,
}

impl BodyRigHint {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, name: impl Into<String>, weight: f32) -> Self {
        self.curves.insert(name.into(), weight);
        self
    }
}

/// One tick of the avatar stream — fully self-describing.
///
/// Field invariants:
///
/// - `weights` is canonical-ordered ARKit (52 floats).
/// - `audio.duration_secs() ≈ 1.0 / timecode.frame_rate` for typical
///   60-Hz operation. The Sync-Manager enforces this.
/// - `emotion` is `Some` only when the harness wants the receiver to
///   blend an emotion overlay on top of the lipsync weights. (Most
///   frames it will be `None` — the harness pre-mixes.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarFrame {
    pub timecode: SmpteTimecode,
    pub audio: AudioChunk,
    pub weights: BlendshapeWeights,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emotion: Option<EmotionVector>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<BodyRigHint>,
}

impl AvatarFrame {
    /// Silent neutral frame at the given timecode — handy for idle
    /// ticks (blinks/saccades) where the harness has no audio to send.
    pub fn neutral(timecode: SmpteTimecode) -> Self {
        Self {
            timecode,
            audio: AudioChunk::empty_voice(),
            weights: BlendshapeWeights::zero(),
            emotion: None,
            body: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timecode_format_pads_two_digits() {
        let tc = SmpteTimecode::from_frame_index(0, 60);
        assert_eq!(tc.format(), "00:00:00:00");
    }

    #[test]
    fn timecode_wraps_at_24h() {
        let one_day_frames = 24 * 3600 * 60;
        let tc = SmpteTimecode::from_frame_index(one_day_frames as u64, 60);
        assert_eq!(tc.format(), "00:00:00:00");
    }

    #[test]
    fn audio_chunk_duration_matches_expected() {
        let mut chunk = AudioChunk::empty_voice();
        chunk.samples_s16le = vec![0u8; 16_000 * 2]; // 1 second of mono s16
        assert!((chunk.duration_secs() - 1.0).abs() < 1e-6);
    }
}
