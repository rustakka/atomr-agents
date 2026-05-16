//! Synthesis actor — turns response text into audio + viseme timing.
//!
//! Backed by any [`atomr_agents_tts_core::TextToSpeech`] implementation.
//! Most TTS backends emit PCM audio plus character-level (ElevenLabs)
//! or phoneme-level (Piper, Azure) alignment; we accept either and
//! convert into a uniform stream of [`VisemeFrame`]s.
//!
//! The synthesis actor itself does *not* know about ARKit; it stops
//! at visemes. The Sync-Manager (next stage) is responsible for the
//! viseme→ARKit overlay, so per-rig overrides live there.

use std::sync::Arc;

use atomr_agents_avatar_core::{AvatarError, Result, Viseme, VisemeFrame};
use atomr_agents_tts_core::{AudioOutput, DynTextToSpeech, SynthesisRequest, VoiceRef};

/// What the synthesis actor produces per utterance.
#[derive(Debug, Clone)]
pub struct SynthesisOutput {
    pub audio: AudioOutput,
    /// Time-ordered visemes covering the utterance. Empty if the
    /// backend didn't emit alignment; callers can fall back to a
    /// constant `Sil` shape or a simple `JawOpen` envelope.
    pub visemes: Vec<VisemeFrame>,
}

/// The synthesis actor. Cheap to clone — holds an `Arc` to the TTS
/// plus the voice it should use.
#[derive(Clone)]
pub struct SynthesisActor {
    tts: DynTextToSpeech,
    voice: VoiceRef,
}

impl SynthesisActor {
    pub fn new(tts: DynTextToSpeech, voice: VoiceRef) -> Self {
        Self { tts, voice }
    }

    /// Drive one utterance. Currently relies on batch synthesis;
    /// streaming/realtime are out of scope for v1 (a future revision
    /// can swap in `synthesize_stream` and chunk the viseme track).
    pub async fn speak(&self, text: &str) -> Result<SynthesisOutput> {
        let request = SynthesisRequest::tts(text.to_string(), self.voice.clone());
        let audio = self
            .tts
            .synthesize(request)
            .await
            .map_err(|e| AvatarError::synthesis(e.to_string()))?;

        // No backend in the matrix surfaces visemes through the
        // `TextToSpeech` batch result today. Phase-2 of the avatar
        // roadmap wires a phonemizer in front of `audio.duration_secs`
        // to emit a synthetic alignment; for now, generate a coarse
        // "open jaw while audio plays" track so the rig still moves.
        let visemes = synthetic_jaw_track(audio.duration_secs);
        Ok(SynthesisOutput { audio, visemes })
    }
}

/// Build a minimal viseme track: alternating `Aa`/`Sil` at 5 Hz for
/// the duration of the audio. This is intentionally crude — it gives
/// the rig something to do while we wait on a real phonemizer-based
/// aligner (phase-2 work documented in the FR for STT/TTS
/// consolidation).
pub(crate) fn synthetic_jaw_track(duration_secs: f32) -> Vec<VisemeFrame> {
    if duration_secs <= 0.0 || !duration_secs.is_finite() {
        return Vec::new();
    }
    let step = 0.1_f32; // 10 fps viseme cadence
    let mut out = Vec::new();
    let mut t = 0.0_f32;
    let mut open = true;
    while t < duration_secs {
        let end = (t + step).min(duration_secs);
        out.push(VisemeFrame {
            viseme: if open { Viseme::Aa } else { Viseme::Sil },
            start_secs: t,
            end_secs: end,
            weight: if open { 0.5 } else { 0.0 },
        });
        open = !open;
        t = end;
    }
    out
}

/// Convenience wrapper that lets a caller construct a [`SynthesisActor`]
/// from an `Arc<dyn TextToSpeech>` produced by any sibling
/// `atomr-agents-tts-runtime-*` crate, plus a [`VoiceRef`].
pub fn synthesis_actor_from_tts(
    tts: Arc<dyn atomr_agents_tts_core::TextToSpeech>,
    voice: VoiceRef,
) -> SynthesisActor {
    SynthesisActor::new(tts, voice)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_track_covers_audio_duration() {
        let track = synthetic_jaw_track(1.0);
        assert!(!track.is_empty());
        let total: f32 = track.iter().map(|v| v.end_secs - v.start_secs).sum();
        assert!((total - 1.0).abs() < 1e-3);
        assert_eq!(track.first().unwrap().start_secs, 0.0);
        assert!((track.last().unwrap().end_secs - 1.0).abs() < 1e-3);
    }

    #[test]
    fn synthetic_track_is_empty_for_zero_audio() {
        assert!(synthetic_jaw_track(0.0).is_empty());
        assert!(synthetic_jaw_track(-1.0).is_empty());
        assert!(synthetic_jaw_track(f32::NAN).is_empty());
    }

    #[test]
    fn synthetic_track_alternates_visemes() {
        let track = synthetic_jaw_track(0.5);
        let visemes: Vec<_> = track.iter().map(|v| v.viseme).collect();
        // First should be open, then alternate.
        assert_eq!(visemes[0], Viseme::Aa);
        for window in visemes.windows(2) {
            assert_ne!(window[0], window[1]);
        }
    }
}
