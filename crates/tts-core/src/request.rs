//! `SynthesisRequest` — covers the five MOSS-TTS surfaces uniformly.
//!
//! - [`SynthesisRequest::Tts`]          — plain text → speech.
//! - [`SynthesisRequest::SoundEffect`]  — text prompt → SFX.
//! - [`SynthesisRequest::Dialogue`]     — multi-speaker script.
//!
//! Voicegen is expressed as `Tts { voice: VoiceRef::DescribedAs(_) }`;
//! voice cloning is `Tts { voice: VoiceRef::ClonedFrom(_) }`. This
//! keeps the request enum compact (3 variants) while the [`VoiceRef`]
//! sum type captures the per-voice strategy.

use serde::Serialize;

use crate::voice::VoiceRef;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SynthesisRequest {
    Tts {
        text: String,
        voice: VoiceRef,
        #[serde(default)]
        options: SynthOptions,
    },
    SoundEffect {
        prompt: String,
        #[serde(default)]
        duration_secs: Option<f32>,
        #[serde(default)]
        options: SynthOptions,
    },
    Dialogue {
        script: Vec<DialogueTurn>,
        speakers: Vec<SpeakerVoice>,
        #[serde(default)]
        options: SynthOptions,
    },
}

impl SynthesisRequest {
    pub fn tts(text: impl Into<String>, voice: VoiceRef) -> Self {
        Self::Tts {
            text: text.into(),
            voice,
            options: SynthOptions::default(),
        }
    }

    pub fn sfx(prompt: impl Into<String>) -> Self {
        Self::SoundEffect {
            prompt: prompt.into(),
            duration_secs: None,
            options: SynthOptions::default(),
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            SynthesisRequest::Tts { .. } => "tts",
            SynthesisRequest::SoundEffect { .. } => "sound_effect",
            SynthesisRequest::Dialogue { .. } => "dialogue",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SynthOptions {
    /// BCP-47 hint. `None` lets the backend autodetect.
    #[serde(default)]
    pub language: Option<String>,
    /// Override the configured model (e.g. `"tts-1-hd"`).
    #[serde(default)]
    pub model: Option<String>,
    /// Free-text style instruction (`gpt-4o-mini-tts`,
    /// ElevenLabs voice settings, MOSS sampling overrides).
    #[serde(default)]
    pub style: Option<String>,
    /// Linear pitch multiplier (1.0 = unchanged).
    #[serde(default)]
    pub pitch: Option<f32>,
    /// Linear rate multiplier (1.0 = unchanged).
    #[serde(default)]
    pub rate: Option<f32>,
    /// Linear volume multiplier (1.0 = unchanged).
    #[serde(default)]
    pub volume: Option<f32>,
    /// Output container preference. Backends that don't support it
    /// fall back to their default.
    #[serde(default)]
    pub format: Option<atomr_agents_stt_core::AudioFormat>,
    /// Backend-specific extras (avoids growing this struct per quirk).
    #[serde(default)]
    pub extra: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DialogueTurn {
    /// Speaker identifier — must match a `SpeakerVoice::tag` below.
    pub speaker: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpeakerVoice {
    /// Tag used in `DialogueTurn::speaker` (e.g. `"S1"`, `"alice"`).
    pub tag: String,
    pub voice: VoiceRef,
}
