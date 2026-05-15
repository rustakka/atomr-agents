//! Transcript output shape.

use serde::{Deserialize, Serialize};

use crate::kinds::BackendKind;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    pub text: String,
    /// BCP-47 if known. `None` if backend didn't return one and
    /// language detection was off.
    pub language: Option<String>,
    pub segments: Vec<Segment>,
    pub duration_secs: f32,
    pub backend: BackendKind,
    pub model_id: Option<String>,
    pub cost_usd: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub text: String,
    pub start_ms: u32,
    pub end_ms: u32,
    pub words: Vec<Word>,
    pub speaker: Option<SpeakerTag>,
    pub confidence: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Word {
    pub text: String,
    pub start_ms: u32,
    pub end_ms: u32,
    pub confidence: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeakerTag {
    pub id: u8,
    pub label: Option<String>,
}

impl Transcript {
    /// Construct a single-segment transcript from text + timing —
    /// useful for backends that only return aggregate text and
    /// duration.
    pub fn from_text(text: impl Into<String>, backend: BackendKind, duration_secs: f32) -> Self {
        let text = text.into();
        let end_ms = (duration_secs * 1000.0) as u32;
        let segment = Segment {
            text: text.clone(),
            start_ms: 0,
            end_ms,
            words: Vec::new(),
            speaker: None,
            confidence: None,
        };
        Self {
            text,
            language: None,
            segments: vec![segment],
            duration_secs,
            backend,
            model_id: None,
            cost_usd: None,
        }
    }
}
