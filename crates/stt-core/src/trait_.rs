//! The central [`SpeechToText`] trait.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::audio::AudioInput;
use crate::capabilities::Capabilities;
use crate::error::Result;
use crate::kinds::{BackendKind, TransportKind};
use crate::stream::{StreamOptions, StreamingSession};
use crate::transcript::Transcript;

/// Per-call options shared across batch transcription backends.
#[derive(Debug, Clone, Default)]
pub struct TranscribeOptions {
    /// BCP-47 language hint. Skipping triggers detection on backends
    /// where `Capabilities::language_detection` is `true`.
    pub language: Option<String>,
    /// Override the configured model (e.g. `"whisper-1"` ↔ `"gpt-4o-transcribe"`).
    pub model: Option<String>,
    pub diarize: bool,
    pub punctuation: bool,
    pub profanity_filter: bool,
    /// Word-level boost / custom vocabulary terms.
    pub keywords: Vec<String>,
    /// System-prompt-style hint (Whisper `prompt`, Deepgram `keywords`).
    pub initial_prompt: Option<String>,
    /// Backend-specific extras (avoids growing this struct per quirk).
    pub extra: Option<serde_json::Value>,
}

/// Speech-to-text backend. Implementations live in sibling
/// `stt-runtime-*` crates.
#[async_trait]
pub trait SpeechToText: Send + Sync + 'static {
    fn capabilities(&self) -> &'static Capabilities;
    fn backend_kind(&self) -> BackendKind;
    fn transport_kind(&self) -> TransportKind;

    /// Single-shot transcription. Backends without batch support
    /// should return [`crate::SttError::UnsupportedCapability`].
    async fn transcribe(
        &self,
        input: AudioInput,
        opts: TranscribeOptions,
    ) -> Result<Transcript>;

    /// Open a streaming-push session. Backends without streaming
    /// return [`crate::SttError::UnsupportedCapability`]; callers
    /// gate via `capabilities().streaming_push` first.
    async fn open_stream(
        &self,
        opts: StreamOptions,
    ) -> Result<Box<dyn StreamingSession>>;
}

pub type DynSpeechToText = Arc<dyn SpeechToText>;

// Marker derives so this lives next to the struct it documents.
// (No code — the `serde` derives keep happy when downstream code
// adds these to typed configs.)
#[allow(dead_code)]
#[derive(Serialize, Deserialize)]
struct _SerdeAnchor;
