//! Output-side audio I/O for atomr-agents text-to-speech.
//!
//! Mirrors `atomr-agents-stt-audio` for the inverse direction:
//! PCM -> container encoders, and (under `speaker`) a `cpal`-based
//! speaker-output stream that mirrors `MicCaptureSession`.
//!
//! Two independent feature surfaces:
//!
//! - `encode-wav` (default-on): `hound`-backed WAV writer.
//! - `speaker` (default-off, needs `libasound2-dev` on Linux):
//!   cpal output stream with a bounded mpsc producer queue.
//!
//! Reuses `AudioFormat`, `PcmBuffer`, `SampleType` from
//! `stt-core` so the two stacks share one vocabulary.

#[cfg(feature = "encode-wav")]
pub mod encode;

#[cfg(feature = "speaker")]
pub mod speaker;

#[cfg(feature = "speaker")]
pub mod pump;
