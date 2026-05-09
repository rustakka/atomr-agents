//! MOSS-TTS backend for atomr-agents.
//!
//! MOSS-TTS is the OpenMOSS family of speech models exposing five
//! capability surfaces (TTS / SFX / dialogue / voicegen / realtime).
//! This crate ships the [`atomr_agents_tts_core::TextToSpeech`] trait
//! surface plus full [`CAPS`] coverage of those five surfaces. The
//! actual model serving runs out-of-process via Python (SGLang or a
//! thin FastAPI wrapper); the `moss-http` Cargo feature enables the
//! HTTP client. Without that feature the runner returns ModelLoad
//! errors so the architecture story stays intact.

mod caps;
mod config;
mod runner;

pub use caps::CAPS;
pub use config::{MossModelVariant, MossTtsConfig};
pub use runner::MossTtsRunner;
