//! Affect state for the avatar.
//!
//! [`EmotionVector`] is a continuous, persistent mood field; agents
//! emit [`EmotionDelta`]s per turn that the [`EmotionVector::apply`]
//! method folds in with simple decay. Downstream, the harness maps the
//! resulting state onto face-board sliders (`mouthSmile*`, `browInnerUp`,
//! `mouthFrown*`, etc.) that ride alongside the lipsync blendshapes.

use serde::{Deserialize, Serialize};

use crate::blendshape::{ArkitBlendshape, BlendshapeWeights};

/// A scalar per-axis affect vector, each component in `[-1.0, 1.0]`
/// except `arousal`/`tension` which are `[0.0, 1.0]`.
///
/// These are *suggestions* the rig can mix in — they are not the
/// blendshape weights themselves. Conversion happens in
/// [`EmotionVector::to_blendshape_overlay`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EmotionVector {
    /// Negative ↔ sadness; positive ↔ joy.
    pub valence: f32,
    /// `0.0` calm ↔ `1.0` excited.
    pub arousal: f32,
    /// `0.0` relaxed ↔ `1.0` angry. Negative is not meaningful.
    pub anger: f32,
    /// `0.0` relaxed ↔ `1.0` surprised.
    pub surprise: f32,
    /// `0.0` relaxed ↔ `1.0` overall facial tension (jaw, brow furrow).
    pub tension: f32,
}

impl EmotionVector {
    /// Neutral / rest state.
    pub const fn neutral() -> Self {
        Self {
            valence: 0.0,
            arousal: 0.0,
            anger: 0.0,
            surprise: 0.0,
            tension: 0.0,
        }
    }

    /// Fold a delta into the running state.
    ///
    /// `decay` ∈ `[0.0, 1.0]` controls how strongly the *prior* value
    /// is retained: `0.0` snaps to the new value, `1.0` ignores the
    /// delta. Typical conversational use is `0.4`–`0.7` — enough
    /// inertia that one-turn surprises don't whiplash the face.
    pub fn apply(&mut self, delta: EmotionDelta, decay: f32) {
        let d = decay.clamp(0.0, 1.0);
        self.valence = (self.valence * d + delta.valence * (1.0 - d)).clamp(-1.0, 1.0);
        self.arousal = (self.arousal * d + delta.arousal * (1.0 - d)).clamp(0.0, 1.0);
        self.anger = (self.anger * d + delta.anger * (1.0 - d)).clamp(0.0, 1.0);
        self.surprise = (self.surprise * d + delta.surprise * (1.0 - d)).clamp(0.0, 1.0);
        self.tension = (self.tension * d + delta.tension * (1.0 - d)).clamp(0.0, 1.0);
    }

    /// Render this affect state to a sparse [`BlendshapeWeights`]
    /// overlay. Only emotion-related shapes are touched — the result
    /// is meant to be max-merged with a lipsync weight vector via
    /// [`BlendshapeWeights::max_merge`] so mouth-position isn't clobbered.
    pub fn to_blendshape_overlay(self) -> BlendshapeWeights {
        let mut w = BlendshapeWeights::zero();

        let smile = self.valence.max(0.0);
        w.set(ArkitBlendshape::MouthSmileLeft, smile);
        w.set(ArkitBlendshape::MouthSmileRight, smile);

        let frown = (-self.valence).max(0.0);
        w.set(ArkitBlendshape::MouthFrownLeft, frown);
        w.set(ArkitBlendshape::MouthFrownRight, frown);

        w.set(ArkitBlendshape::BrowInnerUp, self.surprise);
        w.set(ArkitBlendshape::EyeWideLeft, self.surprise);
        w.set(ArkitBlendshape::EyeWideRight, self.surprise);

        let anger = self.anger;
        w.set(ArkitBlendshape::BrowDownLeft, anger);
        w.set(ArkitBlendshape::BrowDownRight, anger);
        w.set(ArkitBlendshape::NoseSneerLeft, anger * 0.7);
        w.set(ArkitBlendshape::NoseSneerRight, anger * 0.7);

        let squint = self.tension * 0.6;
        w.set(ArkitBlendshape::EyeSquintLeft, squint);
        w.set(ArkitBlendshape::EyeSquintRight, squint);

        w
    }
}

impl Default for EmotionVector {
    fn default() -> Self {
        Self::neutral()
    }
}

/// Per-turn change suggested by the cognition layer. Same axis
/// semantics as [`EmotionVector`].
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct EmotionDelta {
    pub valence: f32,
    pub arousal: f32,
    pub anger: f32,
    pub surprise: f32,
    pub tension: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_with_zero_decay_snaps_to_delta() {
        let mut e = EmotionVector::neutral();
        e.apply(
            EmotionDelta {
                valence: 0.8,
                arousal: 0.5,
                anger: 0.0,
                surprise: 0.0,
                tension: 0.0,
            },
            0.0,
        );
        assert!((e.valence - 0.8).abs() < 1e-6);
        assert!((e.arousal - 0.5).abs() < 1e-6);
    }

    #[test]
    fn apply_with_full_decay_ignores_delta() {
        let mut e = EmotionVector {
            valence: 0.2,
            arousal: 0.0,
            anger: 0.0,
            surprise: 0.0,
            tension: 0.0,
        };
        e.apply(
            EmotionDelta {
                valence: -1.0,
                ..Default::default()
            },
            1.0,
        );
        assert!((e.valence - 0.2).abs() < 1e-6);
    }

    #[test]
    fn positive_valence_only_drives_smile_not_frown() {
        let mut e = EmotionVector::neutral();
        e.valence = 0.6;
        let w = e.to_blendshape_overlay();
        assert!(w.get(ArkitBlendshape::MouthSmileLeft) > 0.0);
        assert_eq!(w.get(ArkitBlendshape::MouthFrownLeft), 0.0);
    }
}
