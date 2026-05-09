//! OpenAI Realtime API backend.
//!
//! `wss://api.openai.com/v1/realtime?model=gpt-4o-realtime-preview`
//! is a bidirectional WebSocket carrying both inbound transcripts
//! and outbound audio (and vice versa). This crate implements the
//! [`atomr_agents_tts_core::TextToSpeech::open_realtime`] surface
//! against that endpoint.
//!
//! The same WebSocket also drives speech-to-text on the inbound
//! side; a follow-up revision will add a parallel
//! [`atomr_agents_stt_core::SpeechToText`] impl on the same runner
//! struct so a single connection serves both directions for the
//! `Conversation` session.

mod caps;
mod config;
mod runner;
mod session;

pub use caps::CAPS;
pub use config::OpenAiRealtimeConfig;
pub use runner::OpenAiRealtimeRunner;
pub use session::OpenAiRealtimeSession;
