//! Core types for the atomr-agents speech-to-text capability.
//!
//! This crate is intentionally I/O-free: it defines the
//! [`SpeechToText`] and [`StreamingSession`] traits, the rich
//! [`Capabilities`] struct that backends advertise via a
//! `pub const`, the audio-input and transcript data types, and a
//! deterministic [`MockSpeechToText`] for tests.
//!
//! Concrete backends live in sibling crates:
//!
//! - `atomr-agents-stt-runtime-openai` — OpenAI Whisper REST.
//! - `atomr-agents-stt-runtime-deepgram` — Deepgram REST + WS.
//! - `atomr-agents-stt-runtime-assemblyai` — AssemblyAI REST + WS.
//! - `atomr-agents-stt-runtime-whisper` — local whisper-rs.
//!
//! Audio I/O (`symphonia`, `cpal`) lives in `atomr-agents-stt-audio`,
//! the higher-level voice-session abstraction in
//! `atomr-agents-stt-voice`, and the agent-framework adapters in
//! `atomr-agents-stt-tool`.

mod audio;
mod capabilities;
mod error;
mod kinds;
mod mock;
mod stream;
mod trait_;
mod transcript;

pub use audio::{AudioFormat, AudioInput, PcmBuffer, SampleType};
pub use capabilities::{Capabilities, DiarizationSupport, Languages};
pub use error::{Result, SttError};
pub use kinds::{BackendKind, TransportKind};
pub use mock::MockSpeechToText;
pub use stream::{StreamEvent, StreamOptions, StreamingSession};
pub use trait_::{DynSpeechToText, SpeechToText, TranscribeOptions};
pub use transcript::{Segment, SpeakerTag, Transcript, Word};
