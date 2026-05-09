//! Deepgram STT backend.
//!
//! Implements [`atomr_agents_stt_core::SpeechToText`] for both
//! batch (REST `POST /v1/listen` with raw audio body) and streaming
//! (WebSocket `wss://api.deepgram.com/v1/listen?…`).

mod caps;
mod config;
mod runner;
mod stream;
mod wire;

pub use caps::CAPS;
pub use config::DeepgramConfig;
pub use runner::DeepgramRunner;
