//! Google Gemini Live API backend.
//!
//! WebSocket endpoint:
//! `wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent`.
//!
//! Implements [`atomr_agents_tts_core::TextToSpeech::open_realtime`].
//! Like the OpenAI Realtime backend, the same WS carries inbound
//! ASR transcripts and outbound audio; a follow-up revision will
//! also implement [`atomr_agents_stt_core::SpeechToText`] on the
//! same runner so the connection serves both directions.

mod caps;
mod config;
mod runner;
mod session;

pub use caps::CAPS;
pub use config::GeminiLiveConfig;
pub use runner::GeminiLiveRunner;
pub use session::GeminiLiveSession;
