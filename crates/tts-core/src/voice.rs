//! Voice reference: pick from a backend library, describe in text
//! (MOSS-VoiceGenerator), or clone from a reference audio sample.

use atomr_agents_stt_core::AudioInput;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VoiceRef {
    /// Backend voice ID from its preset library
    /// (e.g. ElevenLabs `"rachel"`, OpenAI `"alloy"`).
    Library { id: String },
    /// MOSS-VoiceGenerator-style: voice described in free text
    /// (e.g. `"calm, deep, with a slight Scottish accent"`).
    DescribedAs { description: String },
    /// Zero-shot voice cloning from a reference clip.
    #[serde(skip)]
    ClonedFrom(AudioInput),
    /// Backend-specific opaque payload (round-tripped as JSON).
    Custom(serde_json::Value),
}

impl VoiceRef {
    pub fn library(id: impl Into<String>) -> Self {
        Self::Library { id: id.into() }
    }

    pub fn described(description: impl Into<String>) -> Self {
        Self::DescribedAs {
            description: description.into(),
        }
    }

    pub fn cloned_from(audio: AudioInput) -> Self {
        Self::ClonedFrom(audio)
    }

    pub fn kind(&self) -> &'static str {
        match self {
            VoiceRef::Library { .. } => "library",
            VoiceRef::DescribedAs { .. } => "described_as",
            VoiceRef::ClonedFrom(_) => "cloned_from",
            VoiceRef::Custom(_) => "custom",
        }
    }
}
