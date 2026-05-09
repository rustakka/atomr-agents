//! Bidirectional realtime session — the contract OpenAI Realtime,
//! Gemini Live, ElevenLabs Conversational AI, and MOSS-TTS-Realtime
//! all implement (with varying capability subsets).
//!
//! Caller can `push_text` (agent-generated reply to vocalise) or
//! `push_audio` (mic input for inbound ASR), and consumes
//! [`RealtimeEvent`]s emitted by the backend.

use std::pin::Pin;

use async_trait::async_trait;
use atomr_agents_stt_core::{Result, SttError};
use bytes::Bytes;
use futures::Stream;
use serde::Serialize;

use crate::capabilities::Capabilities;
use crate::stream::{AudioChunk, WordTiming};

#[derive(Debug, Clone, Default)]
pub struct RealtimeOptions {
    /// Voice ID to use for outbound audio (when the backend has a
    /// preset library, e.g. OpenAI Realtime: `"alloy"`).
    pub voice_id: Option<String>,
    /// System / instructions message for the conversation context.
    pub instructions: Option<String>,
    /// BCP-47 hint for ASR side.
    pub language: Option<String>,
    /// Sampling temperature passed to the underlying model
    /// (`OpenAI Realtime` accepts 0.6–1.2).
    pub temperature: Option<f32>,
    /// Backend-specific extras.
    pub extra: Option<serde_json::Value>,
}

#[async_trait]
pub trait RealtimeSession: Send {
    fn capabilities(&self) -> &'static Capabilities;

    /// Push a text turn to be vocalised by the backend.
    async fn push_text(&mut self, text: &str) -> Result<()>;

    /// Push raw audio (mic input) for inbound transcription.
    async fn push_audio(&mut self, chunk: Bytes) -> Result<()>;

    /// Signal end-of-user-turn (some backends need it explicitly).
    async fn commit_input(&mut self) -> Result<()>;

    /// Cancel current assistant playback (barge-in).
    async fn interrupt(&mut self) -> Result<()>;

    async fn close(&mut self) -> Result<()>;

    /// Stream of events (audio out, transcripts, VAD signals).
    fn events(
        &mut self,
    ) -> Pin<Box<dyn Stream<Item = std::result::Result<RealtimeEvent, SttError>> + Send + '_>>;
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RealtimeEvent {
    /// Audio frame emitted by the assistant.
    AudioOut { chunk: AudioChunk },
    /// Inbound transcript (user speech ASR).
    InboundTranscript { text: String, is_final: bool },
    /// Outbound text the assistant is about to / just did say.
    OutboundText { text: String, is_final: bool },
    /// Aligned word timing for the most recent outbound utterance.
    OutboundWords { words: Vec<WordTiming> },
    UserSpeechStarted,
    UserSpeechEnded,
    /// User barged in; current assistant turn was cancelled.
    BargeIn,
    /// Assistant turn finished cleanly.
    Done,
    /// Backend-specific metadata blob.
    Metadata(serde_json::Value),
}
