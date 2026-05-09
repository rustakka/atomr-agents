//! OpenAI Whisper / `gpt-4o-transcribe` REST backend.
//!
//! Implements [`atomr_agents_stt_core::SpeechToText`] for batch
//! transcription via `POST /v1/audio/transcriptions`. Streaming /
//! Realtime API is intentionally out of scope here — that lives in a
//! future `stt-runtime-openai-realtime` sibling crate.

mod config;
mod runner;
mod wire;

pub use config::{OpenAiSttConfig, OPENAI_BASE_URL};
pub use runner::{OpenAiSttRunner, CAPS};
