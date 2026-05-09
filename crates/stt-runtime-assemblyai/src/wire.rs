// Wire structs intentionally hold every field we deserialize, even
// the ones we don't currently surface — future feature work can
// pull them out without re-deriving the deserializer.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub(crate) struct UploadResponse {
    pub upload_url: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct CreateTranscriptRequest<'a> {
    pub audio_url: &'a str,
    pub speech_model: Option<&'a str>,
    pub language_code: Option<&'a str>,
    pub speaker_labels: bool,
    pub punctuate: bool,
    pub format_text: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TranscriptStub {
    pub id: String,
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TranscriptResult {
    pub id: String,
    pub status: String,
    pub text: Option<String>,
    pub audio_duration: Option<f32>,
    pub language_code: Option<String>,
    pub words: Option<Vec<AssemblyWord>>,
    pub utterances: Option<Vec<Utterance>>,
    pub error: Option<String>,
    pub speech_model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AssemblyWord {
    pub text: String,
    pub start: u32,
    pub end: u32,
    pub confidence: Option<f32>,
    pub speaker: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Utterance {
    pub text: String,
    pub start: u32,
    pub end: u32,
    pub confidence: Option<f32>,
    pub speaker: Option<String>,
    pub words: Option<Vec<AssemblyWord>>,
}

// --- streaming ---

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum StreamingMessage {
    Begin {
        id: Option<String>,
        expires_at: Option<u64>,
    },
    Turn(TurnMessage),
    Termination {
        audio_duration_seconds: Option<f32>,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TurnMessage {
    pub turn_order: i32,
    pub turn_is_formatted: bool,
    pub end_of_turn: bool,
    pub transcript: String,
    pub end_of_turn_confidence: Option<f32>,
    #[serde(default)]
    pub words: Vec<TurnWord>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TurnWord {
    pub text: String,
    pub start: u32,
    pub end: u32,
    pub confidence: Option<f32>,
    pub word_is_final: Option<bool>,
}
