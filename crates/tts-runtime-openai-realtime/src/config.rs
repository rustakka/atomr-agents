use atomr_agents_stt_remote_core::{SecretRef, Timeouts};
use serde::{Deserialize, Serialize};
use url::Url;

pub const OPENAI_REALTIME_WS_BASE: &str = "wss://api.openai.com/v1/realtime";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiRealtimeConfig {
    /// Base WebSocket URL.
    #[serde(default = "default_endpoint")]
    pub endpoint: Url,
    pub api_key: SecretRef,
    /// Realtime model id (`gpt-4o-realtime-preview`,
    /// `gpt-4o-mini-realtime-preview`, …).
    #[serde(default = "default_model")]
    pub model: String,
    /// Default voice (one of [`crate::caps::OPENAI_REALTIME_VOICES`]).
    #[serde(default = "default_voice")]
    pub default_voice: String,
    /// Optional system prompt seeded into `session.update` on connect.
    #[serde(default)]
    pub instructions: Option<String>,
    /// Modalities to request (default `["audio", "text"]`).
    #[serde(default = "default_modalities")]
    pub modalities: Vec<String>,
    /// Output sample rate (24000 PCM is the API default).
    #[serde(default = "default_sample_rate")]
    pub output_sample_rate: u32,
    #[serde(default)]
    pub timeouts: Timeouts,
}

fn default_endpoint() -> Url {
    Url::parse(OPENAI_REALTIME_WS_BASE).expect("OPENAI_REALTIME_WS_BASE")
}
fn default_model() -> String {
    "gpt-4o-realtime-preview".to_string()
}
fn default_voice() -> String {
    "alloy".to_string()
}
fn default_modalities() -> Vec<String> {
    vec!["audio".to_string(), "text".to_string()]
}
fn default_sample_rate() -> u32 {
    24_000
}

impl OpenAiRealtimeConfig {
    pub fn from_env() -> Self {
        Self {
            endpoint: default_endpoint(),
            api_key: SecretRef::env("OPENAI_API_KEY"),
            model: default_model(),
            default_voice: default_voice(),
            instructions: None,
            modalities: default_modalities(),
            output_sample_rate: default_sample_rate(),
            timeouts: Timeouts::default(),
        }
    }
}
