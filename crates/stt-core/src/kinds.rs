//! Backend / transport discriminator enums.
//!
//! These are intentionally `#[non_exhaustive]` so adding a new
//! `Custom` backend or a `Hybrid` transport later doesn't require a
//! breaking change.

use std::borrow::Cow;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BackendKind {
    OpenAi,
    Deepgram,
    AssemblyAi,
    WhisperLocal,
    Custom(Cow<'static, str>),
}

impl BackendKind {
    pub fn as_str(&self) -> &str {
        match self {
            BackendKind::OpenAi => "openai",
            BackendKind::Deepgram => "deepgram",
            BackendKind::AssemblyAi => "assemblyai",
            BackendKind::WhisperLocal => "whisper_local",
            BackendKind::Custom(name) => name.as_ref(),
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "openai" => BackendKind::OpenAi,
            "deepgram" => BackendKind::Deepgram,
            "assemblyai" => BackendKind::AssemblyAi,
            "whisper_local" => BackendKind::WhisperLocal,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TransportKind {
    /// Synchronous HTTP request/response (REST, multipart upload).
    Rest,
    /// Bidirectional WebSocket / gRPC stream.
    WebSocket,
    /// Local model loaded into the process (no network).
    LocalModel,
    /// Mixes REST control plane with a streaming data plane.
    Hybrid,
}

impl TransportKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TransportKind::Rest => "rest",
            TransportKind::WebSocket => "websocket",
            TransportKind::LocalModel => "local_model",
            TransportKind::Hybrid => "hybrid",
        }
    }

    pub fn requires_network(&self) -> bool {
        !matches!(self, TransportKind::LocalModel)
    }
}
