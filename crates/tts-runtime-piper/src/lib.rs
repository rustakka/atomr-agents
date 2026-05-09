//! Piper ONNX TTS backend.
//!
//! Piper is a fast, small (~50 MB) on-device TTS engine that runs
//! ONNX models. This crate exposes the [`atomr_agents_tts_core::TextToSpeech`]
//! trait surface plus [`CAPS`], with the actual ORT pipeline gated
//! behind the `piper-ort` Cargo feature. Without that feature, the
//! constructor succeeds but [`PiperRunner::synthesize`] returns a
//! typed [`atomr_agents_stt_core::SttError::ModelLoad`] explaining
//! which feature to enable.

mod caps;
mod config;
mod runner;

pub use caps::CAPS;
pub use config::{PiperConfig, PiperVoiceModel};
pub use runner::PiperRunner;
