//! Shared infrastructure for atomr-agents STT cloud backends:
//!
//! - [`SecretRef`] — env / literal / file API-key reference.
//! - [`RetryPolicy`], [`Timeouts`], [`RateLimits`] — shared config shapes.
//! - [`build_http_client`] — `reqwest::Client` builder pre-configured
//!   with timeouts and a TLS backend selected via Cargo features.
//! - [`retry`] — generic exponential-backoff helper around any async
//!   fallible call, classifying retriable errors via [`classify_status`].
//! - [`ws::connect`] — `tokio-tungstenite` connect helper that lifts
//!   `tungstenite::Error` into `SttError::Transport`.
//!
//! Any backend that talks to a cloud API depends on this crate;
//! [`atomr-agents-stt-runtime-whisper`] and the diarizer crate do not.

mod client;
mod config;
mod retry_;

#[cfg(feature = "ws")]
pub mod ws;

pub use client::{build_http_client, classify_status, multipart_filename_for};
pub use config::{RateLimits, RetryPolicy, SecretRef, Timeouts};
pub use retry_::retry;
