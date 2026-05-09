//! On-the-wire JSON shapes. Deepgram returns a deeply-nested
//! envelope; we only depend on the fields we actually map into our
//! [`Transcript`] / [`StreamEvent`]. Extra fields are kept on the
//! structs (and silenced via `dead_code`) so future feature work
//! can pull them out without re-deriving the deserializer.

#![allow(dead_code)]

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct ListenResponse {
    pub results: Option<ListenResults>,
    pub metadata: Option<Metadata>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct Metadata {
    pub duration: Option<f32>,
    pub model_info: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ListenResults {
    pub channels: Vec<Channel>,
    pub utterances: Option<Vec<Utterance>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Channel {
    pub alternatives: Vec<Alternative>,
    pub detected_language: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Alternative {
    pub transcript: String,
    pub confidence: Option<f32>,
    #[serde(default)]
    pub words: Vec<DgWord>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DgWord {
    pub word: String,
    pub start: f32,
    pub end: f32,
    pub confidence: Option<f32>,
    pub speaker: Option<u8>,
    pub punctuated_word: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Utterance {
    pub start: f32,
    pub end: f32,
    pub transcript: String,
    pub confidence: Option<f32>,
    pub speaker: Option<u8>,
    #[serde(default)]
    pub words: Vec<DgWord>,
}

// --- streaming envelopes ---

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum WsMessage {
    Results(ResultsMessage),
    Metadata(MetadataMessage),
    SpeechStarted(SpeechStartedMessage),
    UtteranceEnd(UtteranceEndMessage),
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResultsMessage {
    pub channel: Channel,
    pub is_final: Option<bool>,
    pub speech_final: Option<bool>,
    pub start: f32,
    pub duration: f32,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MetadataMessage {
    pub request_id: Option<String>,
    pub model_info: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SpeechStartedMessage {
    pub timestamp: Option<f32>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UtteranceEndMessage {
    pub last_word_end: Option<f32>,
}
