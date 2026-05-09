//! Local speaker diarization for atomr-agents speech-to-text.
//!
//! Two surfaces:
//!
//! - The [`Diarizer`] trait — async, takes a [`PcmBuffer`] and
//!   returns a list of [`DiarizationSpan`]s with speaker IDs.
//!   Backends whose [`Capabilities::diarization`](atomr_agents_stt_core::Capabilities)
//!   is `None` (notably whisper.cpp and OpenAI Whisper) can layer
//!   this on top to produce diarized transcripts.
//! - [`apply_to_transcript`] — convenience that maps speaker
//!   spans into the [`Segment::speaker`](atomr_agents_stt_core::Segment)
//!   field of an existing [`Transcript`](atomr_agents_stt_core::Transcript).
//!
//! Two implementations:
//!
//! - [`MockDiarizer`] — always-on, deterministic, alternates speaker
//!   IDs every N seconds. Used by tests and downstream demos.
//! - [`SherpaDiarizer`] (`sherpa-onnx` feature) — wraps the
//!   sherpa-onnx speaker-diarization pipeline. Without the feature,
//!   `SherpaDiarizer::new` returns
//!   [`SttError::ModelLoad`](atomr_agents_stt_core::SttError) naming
//!   the missing feature.

mod apply;
mod mock;
mod sherpa;
mod span;

#[cfg(feature = "download-models")]
pub mod download;

pub use apply::apply_to_transcript;
pub use mock::MockDiarizer;
pub use sherpa::{SherpaDiarizer, SherpaDiarizerConfig};
pub use span::{DiarizationSpan, Diarizer};
