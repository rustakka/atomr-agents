//! Streaming-push session abstraction.

use std::pin::Pin;

use async_trait::async_trait;
use bytes::Bytes;
use futures::Stream;
use serde::{Deserialize, Serialize};

use crate::audio::AudioFormat;
use crate::capabilities::Capabilities;
use crate::error::{Result, SttError};
use crate::transcript::{Segment, SpeakerTag, Word};

/// Per-call options for opening a streaming session. Mirrors the
/// "common knobs" across the four MVP backends.
#[derive(Debug, Clone, Default)]
pub struct StreamOptions {
    /// Audio format the caller intends to push. Backends use this to
    /// negotiate the WS handshake (`encoding=…&sample_rate=…`).
    pub format: Option<AudioFormat>,
    /// BCP-47 hint (`"en-US"`). `None` triggers detection on backends
    /// that support it.
    pub language: Option<String>,
    /// Request diarization on backends whose CAPS support it.
    pub diarize: bool,
    /// Model override (e.g. Deepgram `"nova-3"`).
    pub model: Option<String>,
    /// Backend-specific extra knobs round-tripped as JSON. Avoids
    /// adding a knob to this struct for every backend quirk.
    pub extra: Option<serde_json::Value>,
}

/// Active streaming session. Caller alternates `push_audio`/`finish`
/// with consuming the stream returned from `events`.
#[async_trait]
pub trait StreamingSession: Send {
    fn capabilities(&self) -> &'static Capabilities;

    async fn push_audio(&mut self, chunk: Bytes) -> Result<()>;

    /// Mark end-of-stream. Backend flushes; `events` then drains.
    async fn finish(&mut self) -> Result<()>;

    /// Forcibly tear down the session (drop the WS, etc.).
    async fn close(&mut self) -> Result<()>;

    /// Stream of partial / final transcripts and metadata events.
    /// Returned as a boxed pinned stream so the trait is dyn-safe.
    fn events(
        &mut self,
    ) -> Pin<Box<dyn Stream<Item = std::result::Result<StreamEvent, SttError>> + Send + '_>>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StreamEvent {
    /// In-progress transcript. May be revised by a later `Final`.
    Partial {
        text: String,
        start_ms: u32,
        end_ms: u32,
        words: Vec<Word>,
    },
    /// Committed segment.
    Final { segment: Segment },
    /// Speaker-change detected at the given offset.
    SpeakerTurn { speaker: SpeakerTag, at_ms: u32 },
    /// VAD-detected end of utterance.
    UtteranceEnd { at_ms: u32 },
    /// Backend-specific metadata blob (round-tripped to Python as a
    /// dict).
    Metadata(serde_json::Value),
}
