//! Avatar domain layer.
//!
//! An **avatar** is a real-time, visually rendered embodiment of an
//! agent — typically an Unreal Engine 5 MetaHuman driven from outside
//! UE by a Rust actor pipeline. This crate is pure types and traits:
//! no I/O, no model calls.
//!
//! The pipeline downstream of this crate is:
//!
//! ```text
//!   perception (STT) ─► cognition (atomr-infer) ─► synthesis (TTS) ─►
//!   viseme→arkit mapping ─► sync-manager (timecode) ─► AvatarSink ─► UE5
//! ```
//!
//! Every [`AvatarFrame`] flowing through the sink carries an SMPTE
//! timecode, a chunk of audio, a 52-element [`BlendshapeWeights`]
//! vector following Apple's canonical ARKit ordering (which MetaHuman
//! expects natively), an optional [`EmotionVector`] for face-board
//! sliders, and optional body-rig hints.
//!
//! The wire format the sink emits is documented in [`wire`] so both
//! the Rust sender and the Unreal Engine receiver plugin agree on one
//! source of truth.

#![forbid(unsafe_code)]

mod blendshape;
mod emotion;
mod error;
mod frame;
mod sink;
mod viseme;
mod wire;

pub use blendshape::{ArkitBlendshape, BlendshapeWeights, ARKIT_BLENDSHAPE_COUNT};
pub use emotion::{EmotionVector, EmotionDelta};
pub use error::{AvatarError, Result};
pub use frame::{AvatarFrame, AudioChunk, BodyRigHint, SmpteTimecode};
pub use sink::{AvatarSink, SinkCapabilities, SinkHandle, SinkKind};
pub use viseme::{Viseme, VisemeFrame, viseme_to_arkit};
pub use wire::{encode_frame, decode_frame, WireFrame, WIRE_FORMAT_VERSION};
