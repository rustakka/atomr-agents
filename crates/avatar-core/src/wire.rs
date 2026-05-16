//! Wire format streamed to the Unreal Engine 5 receiver plugin.
//!
//! # Framing
//!
//! Every datagram is one CBOR-encoded [`WireFrame`] prefixed by a
//! 4-byte little-endian length header:
//!
//! ```text
//!   ┌────┬───────────────────────────┐
//!   │ N  │     CBOR(WireFrame)       │
//!   └────┴───────────────────────────┘
//!     4B            N bytes
//! ```
//!
//! The length prefix lets a TCP variant of the transport reuse the
//! exact same `encode_frame` function. For UDP transports the prefix
//! is redundant but harmless — keeping the framing consistent means
//! the UE receiver plugin only implements one parser.
//!
//! # Versioning
//!
//! The first field of [`WireFrame`] is `version`. Bumps are
//! backward-compatible: receivers ignore unknown fields, but a major
//! restructure requires bumping [`WIRE_FORMAT_VERSION`] and gating in
//! the UE plugin.
//!
//! # CBOR rationale
//!
//! - Compact (~1 byte overhead per field).
//! - First-class binary type for PCM audio without base64 expansion.
//! - Schema-free (we evolve via optional fields).
//! - UE5 has solid C++ libraries available; `cbor11` is enough.

use serde::{Deserialize, Serialize};

use crate::blendshape::BlendshapeWeights;
use crate::emotion::EmotionVector;
use crate::error::{AvatarError, Result};
use crate::frame::{AudioChunk, AvatarFrame, BodyRigHint, SmpteTimecode};

/// Current wire-format version. Increment only on breaking changes.
pub const WIRE_FORMAT_VERSION: u16 = 1;

/// Length-prefix size in bytes (32-bit unsigned little-endian).
pub const LENGTH_PREFIX_BYTES: usize = 4;

/// The on-wire envelope. Field ordering is stable for forward compat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireFrame {
    /// Bumps in lockstep with [`WIRE_FORMAT_VERSION`].
    pub version: u16,
    pub timecode: SmpteTimecode,
    pub audio: AudioChunk,
    /// All 52 ARKit weights, canonical order.
    pub weights: BlendshapeWeights,
    /// Optional — only present when the harness wants the receiver to
    /// blend an emotion overlay on top of the lipsync.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emotion: Option<EmotionVector>,
    /// Optional — only present when the harness has body-rig hints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<BodyRigHint>,
}

impl From<AvatarFrame> for WireFrame {
    fn from(frame: AvatarFrame) -> Self {
        Self {
            version: WIRE_FORMAT_VERSION,
            timecode: frame.timecode,
            audio: frame.audio,
            weights: frame.weights,
            emotion: frame.emotion,
            body: frame.body,
        }
    }
}

impl From<WireFrame> for AvatarFrame {
    fn from(wire: WireFrame) -> Self {
        AvatarFrame {
            timecode: wire.timecode,
            audio: wire.audio,
            weights: wire.weights,
            emotion: wire.emotion,
            body: wire.body,
        }
    }
}

/// Encode `frame` to the framed wire format.
///
/// Output layout: `[u32 length LE][CBOR bytes]`. Returned `Vec<u8>` is
/// ready to ship out a UDP socket (`socket.send(&bytes)`) or a TCP
/// stream (the prefix self-delimits messages).
pub fn encode_frame(frame: &AvatarFrame) -> Result<Vec<u8>> {
    let wire: WireFrame = frame.clone().into();
    let mut cbor = Vec::with_capacity(256);
    ciborium::ser::into_writer(&wire, &mut cbor)
        .map_err(|e| AvatarError::encode(format!("cbor: {e}")))?;
    let len = u32::try_from(cbor.len())
        .map_err(|_| AvatarError::encode("frame larger than u32::MAX bytes"))?;
    let mut out = Vec::with_capacity(LENGTH_PREFIX_BYTES + cbor.len());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&cbor);
    Ok(out)
}

/// Decode a single framed datagram back into an [`AvatarFrame`].
///
/// Validates the length prefix but does not require the full buffer
/// to be exactly `4 + len` bytes — trailing bytes are tolerated to
/// make this easier to drive from a TCP reader that may have read
/// ahead.
pub fn decode_frame(buf: &[u8]) -> Result<AvatarFrame> {
    if buf.len() < LENGTH_PREFIX_BYTES {
        return Err(AvatarError::decode("buffer shorter than length prefix"));
    }
    let len_bytes: [u8; 4] = buf[..LENGTH_PREFIX_BYTES]
        .try_into()
        .map_err(|e| AvatarError::decode(format!("length prefix: {e}")))?;
    let len = u32::from_le_bytes(len_bytes) as usize;
    let body = buf
        .get(LENGTH_PREFIX_BYTES..LENGTH_PREFIX_BYTES + len)
        .ok_or_else(|| AvatarError::decode("buffer shorter than length-prefixed body"))?;
    let wire: WireFrame = ciborium::de::from_reader(body)
        .map_err(|e| AvatarError::decode(format!("cbor: {e}")))?;
    if wire.version != WIRE_FORMAT_VERSION {
        return Err(AvatarError::decode(format!(
            "unsupported wire version {} (expected {WIRE_FORMAT_VERSION})",
            wire.version
        )));
    }
    Ok(wire.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blendshape::ArkitBlendshape;

    fn sample_frame() -> AvatarFrame {
        let mut weights = BlendshapeWeights::zero();
        weights.set(ArkitBlendshape::JawOpen, 0.75);
        weights.set(ArkitBlendshape::MouthSmileLeft, 0.3);
        AvatarFrame {
            timecode: SmpteTimecode::from_frame_index(180, 60),
            audio: AudioChunk {
                samples_s16le: vec![1, 2, 3, 4, 5, 6, 7, 8],
                sample_rate_hz: 16_000,
                channels: 1,
            },
            weights,
            emotion: Some(EmotionVector {
                valence: 0.4,
                arousal: 0.2,
                anger: 0.0,
                surprise: 0.1,
                tension: 0.0,
            }),
            body: Some(BodyRigHint::new().with("CTRL_pose_lean", 0.2)),
        }
    }

    #[test]
    fn roundtrip_preserves_frame() {
        let frame = sample_frame();
        let bytes = encode_frame(&frame).expect("encode");
        let back = decode_frame(&bytes).expect("decode");
        assert_eq!(back.timecode, frame.timecode);
        assert_eq!(back.audio.samples_s16le, frame.audio.samples_s16le);
        assert_eq!(
            back.weights.get(ArkitBlendshape::JawOpen),
            frame.weights.get(ArkitBlendshape::JawOpen)
        );
        assert_eq!(back.emotion.unwrap().valence, 0.4);
        assert!(back.body.unwrap().curves.contains_key("CTRL_pose_lean"));
    }

    #[test]
    fn length_prefix_matches_payload() {
        let frame = sample_frame();
        let bytes = encode_frame(&frame).expect("encode");
        let len = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
        assert_eq!(len, bytes.len() - LENGTH_PREFIX_BYTES);
    }

    #[test]
    fn short_buffer_decode_errors() {
        assert!(decode_frame(&[]).is_err());
        assert!(decode_frame(&[0, 0, 0]).is_err());
        assert!(decode_frame(&[10, 0, 0, 0, 0, 0]).is_err()); // says 10 bytes follow, but only 2 do
    }

    #[test]
    fn version_mismatch_decode_errors() {
        let mut bogus = WireFrame {
            version: 99,
            timecode: SmpteTimecode::from_frame_index(0, 60),
            audio: AudioChunk::empty_voice(),
            weights: BlendshapeWeights::zero(),
            emotion: None,
            body: None,
        };
        bogus.version = 99;
        let mut cbor = Vec::new();
        ciborium::ser::into_writer(&bogus, &mut cbor).unwrap();
        let mut framed = Vec::new();
        framed.extend_from_slice(&(cbor.len() as u32).to_le_bytes());
        framed.extend_from_slice(&cbor);
        assert!(decode_frame(&framed).is_err());
    }
}
