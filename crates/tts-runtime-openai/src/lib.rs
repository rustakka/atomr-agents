//! OpenAI TTS REST backend.
//!
//! Implements [`atomr_agents_tts_core::TextToSpeech`] for the
//! `POST /v1/audio/speech` endpoint with batch and streaming
//! responses. Models: `tts-1`, `tts-1-hd`, and `gpt-4o-mini-tts`
//! (steerable via the `instructions` body field).
//!
//! Realtime is intentionally out of scope here — that lives in the
//! sibling `atomr-agents-tts-runtime-openai-realtime` crate which
//! talks the WS realtime API.

mod caps;
mod config;
mod runner;
mod stream;

pub use caps::CAPS;
pub use config::{OpenAiTtsConfig, OPENAI_BASE_URL};
pub use runner::OpenAiTtsRunner;
