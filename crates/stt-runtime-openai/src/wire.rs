//! On-the-wire payloads for `POST /v1/audio/transcriptions`. We
//! deliberately use `serde_json::Value` for the `verbose_json`
//! response so we don't break when OpenAI adds fields.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct VerboseTranscription {
    pub text: String,
    pub language: Option<String>,
    pub duration: Option<f32>,
    #[serde(default)]
    pub segments: Vec<VerboseSegment>,
    #[serde(default)]
    pub words: Vec<VerboseWord>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct VerboseSegment {
    pub start: f32,
    pub end: f32,
    pub text: String,
    #[serde(default)]
    pub avg_logprob: Option<f32>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct VerboseWord {
    pub word: String,
    pub start: f32,
    pub end: f32,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum ApiError {
    Wrapped { error: ApiErrorBody },
    Bare(ApiErrorBody),
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApiErrorBody {
    pub message: String,
}

impl ApiError {
    pub fn message(&self) -> &str {
        match self {
            ApiError::Wrapped { error } => &error.message,
            ApiError::Bare(b) => &b.message,
        }
    }
}
