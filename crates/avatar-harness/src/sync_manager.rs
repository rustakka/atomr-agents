//! Sync-manager — bundles PCM audio + viseme track + emotion overlay
//! into timecode-stamped [`AvatarFrame`]s, then ships them out to the
//! attached [`AvatarSink`].
//!
//! Frame cadence is configured per-session (default 60 Hz). Each tick
//! the manager:
//!
//! 1. Slices the next `1 / fps` seconds of PCM audio.
//! 2. Picks the viseme covering this tick and converts it (and any
//!    overlap) to a [`BlendshapeWeights`] overlay.
//! 3. Max-merges the emotion overlay on top.
//! 4. Stamps the result with an SMPTE timecode and pushes it onto the
//!    sink's frame channel.

use atomr_agents_avatar_core::{
    viseme_to_arkit, AudioChunk, AvatarFrame, BlendshapeWeights, SmpteTimecode, Viseme,
    VisemeFrame,
};
use atomr_agents_tts_core::AudioOutput;

use crate::emotion::EmotionState;

/// Sync-manager configuration.
#[derive(Debug, Clone, Copy)]
pub struct SyncConfig {
    /// Output frame rate, in Hz. Common: 30 (Audio2Face), 60 (Live
    /// Link). MUST match what the receiver plugin expects.
    pub frame_rate: u8,
    /// Whether to merge the running emotion overlay onto each frame.
    pub apply_emotion: bool,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            frame_rate: 60,
            apply_emotion: true,
        }
    }
}

/// One utterance's worth of synchronized input.
#[derive(Debug, Clone)]
pub struct SyncBundle {
    pub audio: AudioOutput,
    pub visemes: Vec<VisemeFrame>,
}

/// Sync-manager. Stateless per-utterance (frame_index resets per
/// utterance to keep the SMPTE counter aligned with the audio); the
/// running emotion state is read each tick from the shared
/// [`EmotionState`].
pub struct SyncManager {
    cfg: SyncConfig,
    emotion: EmotionState,
}

impl SyncManager {
    pub fn new(cfg: SyncConfig, emotion: EmotionState) -> Self {
        Self { cfg, emotion }
    }

    /// Produce all [`AvatarFrame`]s for one synthesized utterance.
    /// The first frame's timecode is `00:00:00:00`; callers that want
    /// continuous session time can re-stamp downstream.
    pub fn build_frames(&self, bundle: SyncBundle) -> Vec<AvatarFrame> {
        let fps = self.cfg.frame_rate.max(1);
        let duration = bundle.audio.duration_secs.max(0.0);
        if duration == 0.0 {
            return Vec::new();
        }

        let pcm_s16 = pcm_f32_to_s16le(&bundle.audio.audio.samples);
        let sample_rate = bundle.audio.audio.sample_rate;
        let channels = bundle.audio.audio.channels.max(1) as u32;
        let bytes_per_frame_pcm = 2 * channels as usize; // s16 = 2 bytes

        let total_frames =
            (duration * fps as f32).ceil().max(1.0) as u64;
        let mut out = Vec::with_capacity(total_frames as usize);

        for i in 0..total_frames {
            let start_secs = i as f32 / fps as f32;
            let end_secs = ((i + 1) as f32 / fps as f32).min(duration);

            let start_sample = (start_secs * sample_rate as f32) as usize * bytes_per_frame_pcm;
            let end_sample = (end_secs * sample_rate as f32) as usize * bytes_per_frame_pcm;
            let slice = pcm_s16
                .get(start_sample..end_sample.min(pcm_s16.len()))
                .unwrap_or(&[])
                .to_vec();

            let weights = self.blendshapes_at(start_secs, end_secs, &bundle.visemes);

            out.push(AvatarFrame {
                timecode: SmpteTimecode::from_frame_index(i, fps),
                audio: AudioChunk {
                    samples_s16le: slice,
                    sample_rate_hz: sample_rate,
                    channels: channels.min(u8::MAX as u32) as u8,
                },
                weights,
                emotion: if self.cfg.apply_emotion {
                    Some(self.emotion.snapshot())
                } else {
                    None
                },
                body: None,
            });
        }

        out
    }

    /// Compute blendshape weights for the time window
    /// `[start_secs, end_secs)`. Any viseme overlapping the window
    /// contributes proportional to its overlap.
    fn blendshapes_at(
        &self,
        start_secs: f32,
        end_secs: f32,
        visemes: &[VisemeFrame],
    ) -> BlendshapeWeights {
        let window = (end_secs - start_secs).max(1e-6);
        let mut accumulated = BlendshapeWeights::zero();
        let mut had_voice = false;

        for v in visemes {
            let overlap = (v.end_secs.min(end_secs) - v.start_secs.max(start_secs)).max(0.0);
            if overlap <= 0.0 {
                continue;
            }
            had_voice = true;
            let fraction = (overlap / window).clamp(0.0, 1.0);
            let overlay = viseme_to_arkit(v.viseme, v.weight * fraction);
            accumulated = accumulated.max_merge(&overlay);
        }

        if !had_voice {
            // Silent gap inside the utterance — keep the rest pose.
            accumulated = viseme_to_arkit(Viseme::Sil, 1.0);
        }

        if self.cfg.apply_emotion {
            let emo = self.emotion.snapshot().to_blendshape_overlay();
            accumulated = accumulated.max_merge(&emo);
        }

        accumulated
    }
}

/// Convert a `Vec<f32>` PCM buffer (the TTS layer's native format)
/// into interleaved little-endian signed 16-bit bytes.
fn pcm_f32_to_s16le(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        let clamped = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        out.extend_from_slice(&clamped.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_avatar_core::{ArkitBlendshape, EmotionVector};
    use atomr_agents_stt_core::PcmBuffer;
    use atomr_agents_tts_core::{AudioOutput, BackendKind};

    fn output_with_seconds(secs: f32) -> AudioOutput {
        let sr = 16_000_u32;
        let samples = vec![0.0_f32; (secs * sr as f32 * 1.0) as usize];
        let pcm = PcmBuffer::new(samples, sr, 1);
        AudioOutput::from_pcm(pcm, BackendKind::Custom(std::borrow::Cow::Borrowed("test")), 0)
    }

    #[test]
    fn empty_audio_produces_no_frames() {
        let mgr = SyncManager::new(SyncConfig::default(), EmotionState::default());
        let bundle = SyncBundle {
            audio: output_with_seconds(0.0),
            visemes: Vec::new(),
        };
        assert!(mgr.build_frames(bundle).is_empty());
    }

    #[test]
    fn frame_count_matches_fps_times_duration() {
        let mgr = SyncManager::new(
            SyncConfig {
                frame_rate: 60,
                apply_emotion: false,
            },
            EmotionState::default(),
        );
        let bundle = SyncBundle {
            audio: output_with_seconds(0.5),
            visemes: vec![VisemeFrame {
                viseme: Viseme::Aa,
                start_secs: 0.0,
                end_secs: 0.5,
                weight: 1.0,
            }],
        };
        let frames = mgr.build_frames(bundle);
        assert_eq!(frames.len(), 30); // 0.5s @ 60fps
        assert!(frames[0].weights.get(ArkitBlendshape::JawOpen) > 0.0);
    }

    #[test]
    fn frames_include_emotion_snapshot_when_enabled() {
        let state = EmotionState::new(
            EmotionVector {
                valence: 0.8,
                arousal: 0.4,
                anger: 0.0,
                surprise: 0.0,
                tension: 0.0,
            },
            0.0,
        );
        let mgr = SyncManager::new(
            SyncConfig {
                frame_rate: 30,
                apply_emotion: true,
            },
            state,
        );
        let bundle = SyncBundle {
            audio: output_with_seconds(0.1),
            visemes: vec![VisemeFrame {
                viseme: Viseme::Sil,
                start_secs: 0.0,
                end_secs: 0.1,
                weight: 0.0,
            }],
        };
        let frames = mgr.build_frames(bundle);
        assert!(!frames.is_empty());
        let f = &frames[0];
        assert!(f.emotion.is_some());
        // Smile should be merged onto the lipsync.
        assert!(f.weights.get(ArkitBlendshape::MouthSmileLeft) > 0.0);
    }

    #[test]
    fn timecodes_count_up() {
        let mgr = SyncManager::new(
            SyncConfig {
                frame_rate: 60,
                apply_emotion: false,
            },
            EmotionState::default(),
        );
        let bundle = SyncBundle {
            audio: output_with_seconds(0.1),
            visemes: vec![],
        };
        let frames = mgr.build_frames(bundle);
        for (i, f) in frames.iter().enumerate() {
            assert_eq!(f.timecode.frames as usize, i);
        }
    }
}
