//! Avatar orchestrator.
//!
//! [`AvatarHarness`] is a supervised actor pipeline that turns voice
//! (or text) input into a timecode-aligned stream of audio + ARKit
//! blendshape frames suitable for driving an Unreal Engine 5
//! MetaHuman over Live Link.
//!
//! The five actors are:
//!
//! - **PerceptionActor** — wraps an existing `atomr-agents-stt-*`
//!   runtime; emits transcribed utterances.
//! - **CognitionActor** — calls atomr-infer's `ModelRunner` with a
//!   persona prompt that asks for a structured JSON envelope
//!   (`{response_text, emotion_delta, gesture}`); emits an
//!   `AgentIntentPacket`.
//! - **SynthesisActor** — calls an `atomr-agents-tts-core::TextToSpeech`
//!   to produce audio + alignment, then maps visemes onto ARKit
//!   blendshapes via [`atomr_agents_avatar_core::viseme_to_arkit`].
//! - **EmotionActor** — folds emotion deltas into a long-running
//!   [`EmotionVector`] and surfaces face-board slider weights.
//! - **SyncManager** — stamps audio chunks + blendshape weights with
//!   SMPTE timecodes, buffers for jitter, and emits
//!   [`AvatarFrame`]s onto the bound [`AvatarSink`].
//!
//! The harness is purely additive on top of the existing STT/TTS
//! ecosystems and on top of `atomr-infer` — it does not re-implement
//! model HTTP/SDK plumbing.

#![forbid(unsafe_code)]

mod builder;
mod cognition;
mod emotion;
mod harness;
mod perception;
mod sync_manager;
mod synthesis;

pub use atomr_agents_avatar_core::{
    ArkitBlendshape, AudioChunk, AvatarError, AvatarFrame, AvatarSink, BlendshapeWeights,
    BodyRigHint, EmotionDelta, EmotionVector, Result, SinkCapabilities, SinkHandle, SinkKind,
    SmpteTimecode, Viseme, VisemeFrame,
};

pub use builder::AvatarHarnessBuilder;
pub use cognition::{
    AgentIntentPacket, AvatarInferenceClient, CognitionActor, CognitionConfig, GestureHint,
};
pub use emotion::EmotionState;
pub use harness::{AvatarHarness, AvatarHarnessConfig};
pub use perception::{PerceptionActor, Utterance};
pub use sync_manager::{SyncBundle, SyncConfig, SyncManager};
pub use synthesis::{synthesis_actor_from_tts, SynthesisActor, SynthesisOutput};

/// Test helpers exposed for downstream integration tests and the
/// xtask smoke binary. Not stable.
#[doc(hidden)]
pub mod test_support {
    pub use crate::harness::test_support::CapturingSink;
}
