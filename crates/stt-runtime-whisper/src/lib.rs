//! Local whisper.cpp backend for atomr-agents speech-to-text.
//!
//! The runtime depends on the C++ whisper.cpp library via the
//! [`whisper-rs`] crate. To keep plain workspace builds C++-free,
//! the actual binding is gated behind the `whisper-cpp` feature.
//! Without the feature the crate still exposes:
//!
//! - [`WhisperConfig`] — the deployment config struct.
//! - [`CAPS`] — the `Capabilities` const.
//! - [`WhisperRunner::new`] — returns a runner whose
//!   [`SpeechToText::transcribe`](atomr_agents_stt_core::SpeechToText::transcribe)
//!   surfaces a typed [`SttError::ModelLoad`](atomr_agents_stt_core::SttError::ModelLoad)
//!   that names the missing feature flag.
//!
//! This way callers can advertise the backend's capabilities and
//! depend on the crate transitively without paying the compile-time
//! cost of cmake + a C++ toolchain unless they actually need it.

mod caps;
mod config;
#[cfg(feature = "download-models")]
pub mod download;
mod runner;

pub use caps::CAPS;
pub use config::{WhisperConfig, WhisperModel};
pub use runner::WhisperRunner;
