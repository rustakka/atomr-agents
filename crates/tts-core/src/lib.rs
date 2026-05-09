//! Core types for the atomr-agents text-to-speech capability.
//!
//! Mirrors `atomr-agents-stt-core` on the inverse direction — text
//! goes in, audio comes out. Defines the [`TextToSpeech`] trait
//! plus [`SynthesisStream`] (chunked output) and [`RealtimeSession`]
//! (bidirectional, for OpenAI Realtime / Gemini Live / ElevenLabs
//! Conversational AI / MOSS-TTS-Realtime). Also defines the rich
//! [`Capabilities`] struct that backends advertise via a
//! `pub const`, mapping the five capability surfaces the user
//! anchored on (plain TTS, voicegen, voice cloning, dialogue, sound
//! effects, realtime).
//!
//! Audio primitives ([`AudioInput`], [`AudioFormat`], [`PcmBuffer`],
//! [`SampleType`], [`Languages`], [`TransportKind`]) are re-exported
//! from `atomr-agents-stt-core` so the two capability stacks share
//! one vocabulary.
//!
//! Concrete backends live in sibling crates:
//!
//! - `atomr-agents-tts-runtime-openai`           — batch + streaming
//! - `atomr-agents-tts-runtime-elevenlabs`       — voice library + cloning + SFX + WS
//! - `atomr-agents-tts-runtime-openai-realtime`  — bidirectional realtime
//! - `atomr-agents-tts-runtime-gemini-live`      — bidirectional realtime
//! - `atomr-agents-tts-runtime-moss`             — MOSS-TTS local (all 5 surfaces)
//! - `atomr-agents-tts-runtime-piper`            — Piper ONNX local
//! - `atomr-agents-tts-runtime-kokoro`           — Kokoro ONNX local
//! - `atomr-agents-tts-runtime-xtts`             — Coqui XTTS v2 local
//!
//! Audio output (encoders, speaker) lives in `atomr-agents-tts-audio`.
//! The higher-level `Conversation` session lives in
//! `atomr-agents-tts-voice`. Agent-framework adapters live in
//! `atomr-agents-tts-tool`.

mod capabilities;
mod kinds;
mod mock;
mod realtime;
mod request;
mod stream;
mod trait_;
mod voice;

pub use capabilities::{
    Capabilities, Gender, VoiceCatalog, VoiceCloningSupport, VoiceDescriptor,
};
pub use kinds::BackendKind;
pub use mock::MockTextToSpeech;
pub use realtime::{RealtimeEvent, RealtimeOptions, RealtimeSession};
pub use request::{DialogueTurn, SpeakerVoice, SynthOptions, SynthesisRequest};
pub use stream::{AudioChunk, SynthesisStream};
pub use trait_::{AudioOutput, DynTextToSpeech, TextToSpeech};
pub use voice::VoiceRef;

// Re-exports from stt-core so the two stacks share one vocabulary.
pub use atomr_agents_stt_core::{
    AudioFormat, AudioInput, Languages, PcmBuffer, Result, SampleType, SttError, TransportKind,
};
