//! Kokoro-82M ONNX TTS backend.
//!
//! Kokoro is an 82M-parameter English TTS model with an Apache-2.0
//! license and a static voice catalog of ~50 voices. This crate
//! exposes the [`atomr_agents_tts_core::TextToSpeech`] trait surface
//! plus [`CAPS`], with the ORT pipeline gated behind the
//! `kokoro-ort` Cargo feature.

mod caps;
mod config;
mod runner;

pub use caps::{CAPS, KOKORO_VOICES};
pub use config::KokoroConfig;
pub use runner::KokoroRunner;
