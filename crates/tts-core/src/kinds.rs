//! TTS backend discriminator. Mirrors `stt_core::BackendKind` —
//! `Serialize`/`Deserialize` are manual so a `Custom("name")`
//! variant round-trips as a flat string instead of a tagged object.

use std::borrow::Cow;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BackendKind {
    ElevenLabs,
    OpenAi,
    OpenAiRealtime,
    GeminiLive,
    MossTts,
    Piper,
    Kokoro,
    XttsV2,
    Custom(Cow<'static, str>),
}

impl BackendKind {
    pub fn as_str(&self) -> &str {
        match self {
            BackendKind::ElevenLabs => "elevenlabs",
            BackendKind::OpenAi => "openai",
            BackendKind::OpenAiRealtime => "openai_realtime",
            BackendKind::GeminiLive => "gemini_live",
            BackendKind::MossTts => "moss_tts",
            BackendKind::Piper => "piper",
            BackendKind::Kokoro => "kokoro",
            BackendKind::XttsV2 => "xtts_v2",
            BackendKind::Custom(name) => name.as_ref(),
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "elevenlabs" => BackendKind::ElevenLabs,
            "openai" => BackendKind::OpenAi,
            "openai_realtime" => BackendKind::OpenAiRealtime,
            "gemini_live" => BackendKind::GeminiLive,
            "moss_tts" => BackendKind::MossTts,
            "piper" => BackendKind::Piper,
            "kokoro" => BackendKind::Kokoro,
            "xtts_v2" => BackendKind::XttsV2,
            other => BackendKind::Custom(Cow::Owned(other.to_string())),
        }
    }
}

impl Serialize for BackendKind {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for BackendKind {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s: String = String::deserialize(de)?;
        Ok(BackendKind::from_str(&s))
    }
}
