//! Viseme model + viseme→ARKit-blendshape mapping.
//!
//! We adopt the **Oculus / Azure 15-viseme set** because:
//!
//! - Microsoft Azure Speech and Meta Oculus LipSync both emit it directly.
//! - It's small enough that mapping each viseme to a sparse
//!   [`BlendshapeWeights`] overlay is hand-tuneable.
//! - Engines (Unreal, Unity, Maya) consume the same vocabulary.
//!
//! Backends that emit phonemes (eSpeak, Piper) phonemize → IPA →
//! viseme upstream of this module; backends that emit alignments
//! (ElevenLabs character-level) map char-windows → phonemes → visemes.
//! Both reach [`viseme_to_arkit`] eventually.

use serde::{Deserialize, Serialize};

use crate::blendshape::{ArkitBlendshape, BlendshapeWeights};

/// One frame of viseme info: which mouth-shape and how strongly to
/// hold it. `start_secs` / `end_secs` are sink-relative (zero-based
/// from the start of the utterance) so the Sync-Manager can window
/// these against TTS audio.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisemeFrame {
    pub viseme: Viseme,
    pub start_secs: f32,
    pub end_secs: f32,
    /// `[0.0, 1.0]` — strength to hold the shape at. Most aligners
    /// only emit on/off, in which case this is `1.0`.
    pub weight: f32,
}

/// Oculus / Azure 15-viseme set. The numeric ordering matches the
/// `VisemeID` Azure emits over its WebSocket, so callers can `as u8`
/// to round-trip.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize,
)]
#[repr(u8)]
pub enum Viseme {
    /// Silence / rest.
    Sil = 0,
    /// `æ`, `ə`, `ʌ` — open neutral "uh / a-like".
    Ae = 1,
    /// `ɑ` — open back "ah" (father).
    Aa = 2,
    /// `ɔ` — rounded back "aw" (saw).
    Ao = 3,
    /// `ɛ`, `ʊ` — front-mid "eh / oo-short".
    Eh = 4,
    /// `ɝ` — r-colored.
    Er = 5,
    /// `j`, `i`, `ɪ` — high front "ee/y".
    Ih = 6,
    /// `w`, `u` — high back rounded "oo/w".
    W = 7,
    /// `o` — mid back rounded "oh".
    Oh = 8,
    /// `s`, `z` — alveolar fricative.
    S = 9,
    /// `ʃ`, `ʒ`, `tʃ`, `dʒ` — post-alveolar.
    Sh = 10,
    /// `θ`, `ð` — dental fricative.
    Th = 11,
    /// `f`, `v` — labio-dental.
    F = 12,
    /// `d`, `t`, `n`, `l` — tongue-tip alveolars.
    D = 13,
    /// `k`, `g`, `ŋ`, `h` — velars + open glottal.
    Kk = 14,
}

impl Viseme {
    /// Stable string id (for JSON / logs).
    pub fn as_str(self) -> &'static str {
        match self {
            Viseme::Sil => "sil",
            Viseme::Ae => "ae",
            Viseme::Aa => "aa",
            Viseme::Ao => "ao",
            Viseme::Eh => "eh",
            Viseme::Er => "er",
            Viseme::Ih => "ih",
            Viseme::W => "w",
            Viseme::Oh => "oh",
            Viseme::S => "s",
            Viseme::Sh => "sh",
            Viseme::Th => "th",
            Viseme::F => "f",
            Viseme::D => "d",
            Viseme::Kk => "kk",
        }
    }

    /// All 15 in canonical id order.
    pub const ALL: [Viseme; 15] = [
        Viseme::Sil,
        Viseme::Ae,
        Viseme::Aa,
        Viseme::Ao,
        Viseme::Eh,
        Viseme::Er,
        Viseme::Ih,
        Viseme::W,
        Viseme::Oh,
        Viseme::S,
        Viseme::Sh,
        Viseme::Th,
        Viseme::F,
        Viseme::D,
        Viseme::Kk,
    ];
}

/// Map a viseme + intensity to a sparse [`BlendshapeWeights`] vector.
///
/// Weights are hand-tuned against the ARKit naming and what
/// MetaHumans render most naturally. They're a starting point —
/// projects with custom faces can override per-viseme overlays.
pub fn viseme_to_arkit(viseme: Viseme, weight: f32) -> BlendshapeWeights {
    let w = weight.clamp(0.0, 1.0);
    let mut out = BlendshapeWeights::zero();

    // Helper closures over the local `out`.
    let mut set = |shape: ArkitBlendshape, value: f32| {
        out.set(shape, value * w);
    };

    match viseme {
        Viseme::Sil => {
            // explicit no-op — keep face at rest.
        }

        Viseme::Ae => {
            set(ArkitBlendshape::JawOpen, 0.35);
            set(ArkitBlendshape::MouthStretchLeft, 0.3);
            set(ArkitBlendshape::MouthStretchRight, 0.3);
        }
        Viseme::Aa => {
            set(ArkitBlendshape::JawOpen, 0.75);
            set(ArkitBlendshape::MouthLowerDownLeft, 0.45);
            set(ArkitBlendshape::MouthLowerDownRight, 0.45);
        }
        Viseme::Ao => {
            set(ArkitBlendshape::JawOpen, 0.55);
            set(ArkitBlendshape::MouthFunnel, 0.55);
            set(ArkitBlendshape::MouthPucker, 0.2);
        }
        Viseme::Eh => {
            set(ArkitBlendshape::JawOpen, 0.3);
            set(ArkitBlendshape::MouthSmileLeft, 0.2);
            set(ArkitBlendshape::MouthSmileRight, 0.2);
            set(ArkitBlendshape::MouthUpperUpLeft, 0.15);
            set(ArkitBlendshape::MouthUpperUpRight, 0.15);
        }
        Viseme::Er => {
            set(ArkitBlendshape::JawOpen, 0.3);
            set(ArkitBlendshape::MouthFunnel, 0.3);
            set(ArkitBlendshape::MouthPucker, 0.3);
        }
        Viseme::Ih => {
            set(ArkitBlendshape::MouthSmileLeft, 0.4);
            set(ArkitBlendshape::MouthSmileRight, 0.4);
            set(ArkitBlendshape::MouthUpperUpLeft, 0.2);
            set(ArkitBlendshape::MouthUpperUpRight, 0.2);
        }
        Viseme::W => {
            set(ArkitBlendshape::MouthPucker, 0.75);
            set(ArkitBlendshape::MouthFunnel, 0.5);
            set(ArkitBlendshape::JawForward, 0.2);
        }
        Viseme::Oh => {
            set(ArkitBlendshape::JawOpen, 0.45);
            set(ArkitBlendshape::MouthFunnel, 0.6);
            set(ArkitBlendshape::MouthPucker, 0.4);
        }
        Viseme::S => {
            set(ArkitBlendshape::MouthSmileLeft, 0.25);
            set(ArkitBlendshape::MouthSmileRight, 0.25);
            set(ArkitBlendshape::MouthClose, 0.4);
            set(ArkitBlendshape::JawOpen, 0.1);
        }
        Viseme::Sh => {
            set(ArkitBlendshape::MouthFunnel, 0.55);
            set(ArkitBlendshape::MouthPucker, 0.5);
            set(ArkitBlendshape::JawOpen, 0.15);
        }
        Viseme::Th => {
            set(ArkitBlendshape::TongueOut, 0.65);
            set(ArkitBlendshape::JawOpen, 0.2);
        }
        Viseme::F => {
            set(ArkitBlendshape::MouthLowerDownLeft, 0.35);
            set(ArkitBlendshape::MouthLowerDownRight, 0.35);
            set(ArkitBlendshape::MouthRollLower, 0.45);
            set(ArkitBlendshape::JawOpen, 0.1);
        }
        Viseme::D => {
            set(ArkitBlendshape::JawOpen, 0.2);
            set(ArkitBlendshape::MouthShrugUpper, 0.2);
            set(ArkitBlendshape::MouthClose, 0.15);
        }
        Viseme::Kk => {
            set(ArkitBlendshape::JawOpen, 0.25);
            set(ArkitBlendshape::MouthShrugLower, 0.2);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_emits_zero_overlay() {
        let w = viseme_to_arkit(Viseme::Sil, 1.0);
        assert!(w.as_array().iter().all(|x| *x == 0.0));
    }

    #[test]
    fn zero_weight_emits_zero_overlay_for_any_viseme() {
        for v in Viseme::ALL.iter().copied() {
            let w = viseme_to_arkit(v, 0.0);
            assert!(w.as_array().iter().all(|x| *x == 0.0), "viseme {:?}", v);
        }
    }

    #[test]
    fn open_visemes_open_the_jaw() {
        for v in [Viseme::Aa, Viseme::Ae, Viseme::Ao, Viseme::Eh, Viseme::Oh] {
            let w = viseme_to_arkit(v, 1.0);
            assert!(
                w.get(ArkitBlendshape::JawOpen) > 0.0,
                "viseme {:?} should open the jaw",
                v
            );
        }
    }

    #[test]
    fn rounded_visemes_drive_pucker() {
        for v in [Viseme::W, Viseme::Oh, Viseme::Ao] {
            let w = viseme_to_arkit(v, 1.0);
            assert!(
                w.get(ArkitBlendshape::MouthPucker) > 0.0
                    || w.get(ArkitBlendshape::MouthFunnel) > 0.0,
                "viseme {:?} should drive rounded mouth",
                v
            );
        }
    }

    #[test]
    fn th_extends_the_tongue() {
        let w = viseme_to_arkit(Viseme::Th, 1.0);
        assert!(w.get(ArkitBlendshape::TongueOut) > 0.0);
    }
}
