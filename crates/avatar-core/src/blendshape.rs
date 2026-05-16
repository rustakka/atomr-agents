//! ARKit 52-blendshape model.
//!
//! Apple's ARKit defines 52 named blendshapes that together describe a
//! human face's expressive range. Unreal Engine 5 MetaHumans ship with
//! a face-rig that consumes this exact set natively, which is why we
//! treat ARKit ordering as canonical at the avatar layer.
//!
//! The numeric ordering of [`ArkitBlendshape`] matches Apple's
//! [official enumeration order][apple] so that `as usize` indexes the
//! corresponding slot in [`BlendshapeWeights`].
//!
//! [apple]: https://developer.apple.com/documentation/arkit/arfaceanchor/blendshapelocation

use serde::{Deserialize, Serialize};

use crate::error::{AvatarError, Result};

/// Number of named blendshapes in Apple's ARKit face model.
pub const ARKIT_BLENDSHAPE_COUNT: usize = 52;

/// The 52 canonical ARKit blendshape locations.
///
/// Variant order matches Apple's documented enum order; `as usize` is
/// the index into a [`BlendshapeWeights`] vector.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize,
)]
#[repr(u8)]
pub enum ArkitBlendshape {
    // Eyes
    EyeBlinkLeft = 0,
    EyeLookDownLeft = 1,
    EyeLookInLeft = 2,
    EyeLookOutLeft = 3,
    EyeLookUpLeft = 4,
    EyeSquintLeft = 5,
    EyeWideLeft = 6,

    EyeBlinkRight = 7,
    EyeLookDownRight = 8,
    EyeLookInRight = 9,
    EyeLookOutRight = 10,
    EyeLookUpRight = 11,
    EyeSquintRight = 12,
    EyeWideRight = 13,

    // Jaw
    JawForward = 14,
    JawLeft = 15,
    JawRight = 16,
    JawOpen = 17,

    // Mouth
    MouthClose = 18,
    MouthFunnel = 19,
    MouthPucker = 20,
    MouthLeft = 21,
    MouthRight = 22,
    MouthSmileLeft = 23,
    MouthSmileRight = 24,
    MouthFrownLeft = 25,
    MouthFrownRight = 26,
    MouthDimpleLeft = 27,
    MouthDimpleRight = 28,
    MouthStretchLeft = 29,
    MouthStretchRight = 30,
    MouthRollLower = 31,
    MouthRollUpper = 32,
    MouthShrugLower = 33,
    MouthShrugUpper = 34,
    MouthPressLeft = 35,
    MouthPressRight = 36,
    MouthLowerDownLeft = 37,
    MouthLowerDownRight = 38,
    MouthUpperUpLeft = 39,
    MouthUpperUpRight = 40,

    // Brows
    BrowDownLeft = 41,
    BrowDownRight = 42,
    BrowInnerUp = 43,
    BrowOuterUpLeft = 44,
    BrowOuterUpRight = 45,

    // Cheeks / nose
    CheekPuff = 46,
    CheekSquintLeft = 47,
    CheekSquintRight = 48,
    NoseSneerLeft = 49,
    NoseSneerRight = 50,

    // Tongue
    TongueOut = 51,
}

impl ArkitBlendshape {
    /// All 52 variants in canonical order.
    pub const ALL: [ArkitBlendshape; ARKIT_BLENDSHAPE_COUNT] = [
        ArkitBlendshape::EyeBlinkLeft,
        ArkitBlendshape::EyeLookDownLeft,
        ArkitBlendshape::EyeLookInLeft,
        ArkitBlendshape::EyeLookOutLeft,
        ArkitBlendshape::EyeLookUpLeft,
        ArkitBlendshape::EyeSquintLeft,
        ArkitBlendshape::EyeWideLeft,
        ArkitBlendshape::EyeBlinkRight,
        ArkitBlendshape::EyeLookDownRight,
        ArkitBlendshape::EyeLookInRight,
        ArkitBlendshape::EyeLookOutRight,
        ArkitBlendshape::EyeLookUpRight,
        ArkitBlendshape::EyeSquintRight,
        ArkitBlendshape::EyeWideRight,
        ArkitBlendshape::JawForward,
        ArkitBlendshape::JawLeft,
        ArkitBlendshape::JawRight,
        ArkitBlendshape::JawOpen,
        ArkitBlendshape::MouthClose,
        ArkitBlendshape::MouthFunnel,
        ArkitBlendshape::MouthPucker,
        ArkitBlendshape::MouthLeft,
        ArkitBlendshape::MouthRight,
        ArkitBlendshape::MouthSmileLeft,
        ArkitBlendshape::MouthSmileRight,
        ArkitBlendshape::MouthFrownLeft,
        ArkitBlendshape::MouthFrownRight,
        ArkitBlendshape::MouthDimpleLeft,
        ArkitBlendshape::MouthDimpleRight,
        ArkitBlendshape::MouthStretchLeft,
        ArkitBlendshape::MouthStretchRight,
        ArkitBlendshape::MouthRollLower,
        ArkitBlendshape::MouthRollUpper,
        ArkitBlendshape::MouthShrugLower,
        ArkitBlendshape::MouthShrugUpper,
        ArkitBlendshape::MouthPressLeft,
        ArkitBlendshape::MouthPressRight,
        ArkitBlendshape::MouthLowerDownLeft,
        ArkitBlendshape::MouthLowerDownRight,
        ArkitBlendshape::MouthUpperUpLeft,
        ArkitBlendshape::MouthUpperUpRight,
        ArkitBlendshape::BrowDownLeft,
        ArkitBlendshape::BrowDownRight,
        ArkitBlendshape::BrowInnerUp,
        ArkitBlendshape::BrowOuterUpLeft,
        ArkitBlendshape::BrowOuterUpRight,
        ArkitBlendshape::CheekPuff,
        ArkitBlendshape::CheekSquintLeft,
        ArkitBlendshape::CheekSquintRight,
        ArkitBlendshape::NoseSneerLeft,
        ArkitBlendshape::NoseSneerRight,
        ArkitBlendshape::TongueOut,
    ];

    /// Apple's canonical name string (camelCase). Useful when emitting
    /// over a JSON wire format or matching against UE5 curve names.
    pub fn as_str(self) -> &'static str {
        match self {
            ArkitBlendshape::EyeBlinkLeft => "eyeBlinkLeft",
            ArkitBlendshape::EyeLookDownLeft => "eyeLookDownLeft",
            ArkitBlendshape::EyeLookInLeft => "eyeLookInLeft",
            ArkitBlendshape::EyeLookOutLeft => "eyeLookOutLeft",
            ArkitBlendshape::EyeLookUpLeft => "eyeLookUpLeft",
            ArkitBlendshape::EyeSquintLeft => "eyeSquintLeft",
            ArkitBlendshape::EyeWideLeft => "eyeWideLeft",
            ArkitBlendshape::EyeBlinkRight => "eyeBlinkRight",
            ArkitBlendshape::EyeLookDownRight => "eyeLookDownRight",
            ArkitBlendshape::EyeLookInRight => "eyeLookInRight",
            ArkitBlendshape::EyeLookOutRight => "eyeLookOutRight",
            ArkitBlendshape::EyeLookUpRight => "eyeLookUpRight",
            ArkitBlendshape::EyeSquintRight => "eyeSquintRight",
            ArkitBlendshape::EyeWideRight => "eyeWideRight",
            ArkitBlendshape::JawForward => "jawForward",
            ArkitBlendshape::JawLeft => "jawLeft",
            ArkitBlendshape::JawRight => "jawRight",
            ArkitBlendshape::JawOpen => "jawOpen",
            ArkitBlendshape::MouthClose => "mouthClose",
            ArkitBlendshape::MouthFunnel => "mouthFunnel",
            ArkitBlendshape::MouthPucker => "mouthPucker",
            ArkitBlendshape::MouthLeft => "mouthLeft",
            ArkitBlendshape::MouthRight => "mouthRight",
            ArkitBlendshape::MouthSmileLeft => "mouthSmileLeft",
            ArkitBlendshape::MouthSmileRight => "mouthSmileRight",
            ArkitBlendshape::MouthFrownLeft => "mouthFrownLeft",
            ArkitBlendshape::MouthFrownRight => "mouthFrownRight",
            ArkitBlendshape::MouthDimpleLeft => "mouthDimpleLeft",
            ArkitBlendshape::MouthDimpleRight => "mouthDimpleRight",
            ArkitBlendshape::MouthStretchLeft => "mouthStretchLeft",
            ArkitBlendshape::MouthStretchRight => "mouthStretchRight",
            ArkitBlendshape::MouthRollLower => "mouthRollLower",
            ArkitBlendshape::MouthRollUpper => "mouthRollUpper",
            ArkitBlendshape::MouthShrugLower => "mouthShrugLower",
            ArkitBlendshape::MouthShrugUpper => "mouthShrugUpper",
            ArkitBlendshape::MouthPressLeft => "mouthPressLeft",
            ArkitBlendshape::MouthPressRight => "mouthPressRight",
            ArkitBlendshape::MouthLowerDownLeft => "mouthLowerDownLeft",
            ArkitBlendshape::MouthLowerDownRight => "mouthLowerDownRight",
            ArkitBlendshape::MouthUpperUpLeft => "mouthUpperUpLeft",
            ArkitBlendshape::MouthUpperUpRight => "mouthUpperUpRight",
            ArkitBlendshape::BrowDownLeft => "browDownLeft",
            ArkitBlendshape::BrowDownRight => "browDownRight",
            ArkitBlendshape::BrowInnerUp => "browInnerUp",
            ArkitBlendshape::BrowOuterUpLeft => "browOuterUpLeft",
            ArkitBlendshape::BrowOuterUpRight => "browOuterUpRight",
            ArkitBlendshape::CheekPuff => "cheekPuff",
            ArkitBlendshape::CheekSquintLeft => "cheekSquintLeft",
            ArkitBlendshape::CheekSquintRight => "cheekSquintRight",
            ArkitBlendshape::NoseSneerLeft => "noseSneerLeft",
            ArkitBlendshape::NoseSneerRight => "noseSneerRight",
            ArkitBlendshape::TongueOut => "tongueOut",
        }
    }

    /// Index into [`BlendshapeWeights`] storage (0..52).
    pub fn index(self) -> usize {
        self as u8 as usize
    }
}

/// 52-element vector of ARKit blendshape weights, each in `[0.0, 1.0]`.
///
/// Layout matches [`ArkitBlendshape::ALL`] — `weights[bs.index()]` is
/// the weight for blendshape `bs`. This mirrors the dense layout
/// MetaHuman face curves consume.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlendshapeWeights(pub [f32; ARKIT_BLENDSHAPE_COUNT]);

impl serde::Serialize for BlendshapeWeights {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeTuple;
        let mut tup = serializer.serialize_tuple(ARKIT_BLENDSHAPE_COUNT)?;
        for v in &self.0 {
            tup.serialize_element(v)?;
        }
        tup.end()
    }
}

impl<'de> serde::Deserialize<'de> for BlendshapeWeights {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor52;
        impl<'de> serde::de::Visitor<'de> for Visitor52 {
            type Value = BlendshapeWeights;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "a sequence of {ARKIT_BLENDSHAPE_COUNT} f32 weights")
            }
            fn visit_seq<A>(self, mut seq: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let mut arr = [0.0_f32; ARKIT_BLENDSHAPE_COUNT];
                for (i, slot) in arr.iter_mut().enumerate() {
                    *slot = seq
                        .next_element::<f32>()?
                        .ok_or_else(|| serde::de::Error::invalid_length(i, &self))?;
                }
                Ok(BlendshapeWeights(arr))
            }
        }
        deserializer.deserialize_tuple(ARKIT_BLENDSHAPE_COUNT, Visitor52)
    }
}

impl BlendshapeWeights {
    /// All zeros — neutral / rest pose.
    pub const fn zero() -> Self {
        Self([0.0; ARKIT_BLENDSHAPE_COUNT])
    }

    /// Construct from a slice; errors if it doesn't have exactly 52 elements.
    pub fn from_slice(values: &[f32]) -> Result<Self> {
        if values.len() != ARKIT_BLENDSHAPE_COUNT {
            return Err(AvatarError::BlendshapeLength {
                expected: ARKIT_BLENDSHAPE_COUNT,
                got: values.len(),
            });
        }
        let mut out = [0.0_f32; ARKIT_BLENDSHAPE_COUNT];
        out.copy_from_slice(values);
        Ok(Self(out))
    }

    /// Read a single weight.
    pub fn get(&self, shape: ArkitBlendshape) -> f32 {
        self.0[shape.index()]
    }

    /// Write a single weight. Values outside `[0.0, 1.0]` are clamped.
    pub fn set(&mut self, shape: ArkitBlendshape, weight: f32) {
        self.0[shape.index()] = weight.clamp(0.0, 1.0);
    }

    /// Clamp every weight to `[0.0, 1.0]` in-place.
    pub fn clamp01(&mut self) {
        for w in &mut self.0 {
            *w = w.clamp(0.0, 1.0);
        }
    }

    /// Linearly interpolate between `self` and `other` by `t ∈ [0, 1]`.
    /// `t` is clamped before use.
    pub fn lerp(&self, other: &Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let mut out = [0.0_f32; ARKIT_BLENDSHAPE_COUNT];
        for i in 0..ARKIT_BLENDSHAPE_COUNT {
            out[i] = self.0[i] * (1.0 - t) + other.0[i] * t;
        }
        Self(out)
    }

    /// Scale every weight by `factor`, clamping to `[0.0, 1.0]`. Useful
    /// for applying Audio2Face's `AnimationHeader.multiplier`.
    pub fn scale(&self, factor: f32) -> Self {
        let mut out = [0.0_f32; ARKIT_BLENDSHAPE_COUNT];
        for i in 0..ARKIT_BLENDSHAPE_COUNT {
            out[i] = (self.0[i] * factor).clamp(0.0, 1.0);
        }
        Self(out)
    }

    /// Element-wise max — useful for merging multiple inputs (e.g.
    /// viseme lips + emotion brows) without one overwriting the other.
    pub fn max_merge(&self, other: &Self) -> Self {
        let mut out = [0.0_f32; ARKIT_BLENDSHAPE_COUNT];
        for i in 0..ARKIT_BLENDSHAPE_COUNT {
            out[i] = self.0[i].max(other.0[i]);
        }
        Self(out)
    }

    /// Borrow the raw `[f32; 52]` storage.
    pub fn as_array(&self) -> &[f32; ARKIT_BLENDSHAPE_COUNT] {
        &self.0
    }
}

impl Default for BlendshapeWeights {
    fn default() -> Self {
        Self::zero()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_names_are_unique_and_canonical() {
        let mut seen = std::collections::HashSet::new();
        for shape in ArkitBlendshape::ALL.iter() {
            assert!(seen.insert(shape.as_str()), "duplicate name {}", shape.as_str());
        }
        assert_eq!(seen.len(), ARKIT_BLENDSHAPE_COUNT);
    }

    #[test]
    fn index_matches_discriminant() {
        for (i, shape) in ArkitBlendshape::ALL.iter().enumerate() {
            assert_eq!(shape.index(), i);
        }
    }

    #[test]
    fn set_clamps_to_unit_range() {
        let mut w = BlendshapeWeights::zero();
        w.set(ArkitBlendshape::JawOpen, 2.5);
        assert_eq!(w.get(ArkitBlendshape::JawOpen), 1.0);
        w.set(ArkitBlendshape::JawOpen, -0.5);
        assert_eq!(w.get(ArkitBlendshape::JawOpen), 0.0);
    }

    #[test]
    fn lerp_endpoints() {
        let mut a = BlendshapeWeights::zero();
        let mut b = BlendshapeWeights::zero();
        a.set(ArkitBlendshape::JawOpen, 0.0);
        b.set(ArkitBlendshape::JawOpen, 1.0);
        assert_eq!(a.lerp(&b, 0.0).get(ArkitBlendshape::JawOpen), 0.0);
        assert_eq!(a.lerp(&b, 1.0).get(ArkitBlendshape::JawOpen), 1.0);
        assert!((a.lerp(&b, 0.5).get(ArkitBlendshape::JawOpen) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn from_slice_length_check() {
        assert!(BlendshapeWeights::from_slice(&[0.0; 51]).is_err());
        assert!(BlendshapeWeights::from_slice(&[0.0; 52]).is_ok());
        assert!(BlendshapeWeights::from_slice(&[0.0; 53]).is_err());
    }

    #[test]
    fn max_merge_picks_larger_per_slot() {
        let mut a = BlendshapeWeights::zero();
        let mut b = BlendshapeWeights::zero();
        a.set(ArkitBlendshape::JawOpen, 0.3);
        b.set(ArkitBlendshape::JawOpen, 0.7);
        a.set(ArkitBlendshape::BrowInnerUp, 0.8);
        b.set(ArkitBlendshape::BrowInnerUp, 0.2);
        let m = a.max_merge(&b);
        assert!((m.get(ArkitBlendshape::JawOpen) - 0.7).abs() < 1e-6);
        assert!((m.get(ArkitBlendshape::BrowInnerUp) - 0.8).abs() < 1e-6);
    }
}
