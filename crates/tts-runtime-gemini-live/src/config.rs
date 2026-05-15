use atomr_agents_stt_remote_core::{SecretRef, Timeouts};
use serde::{Deserialize, Serialize};
use url::Url;

pub const GEMINI_LIVE_WS_BASE: &str =
    "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiLiveConfig {
    /// WebSocket endpoint.
    #[serde(default = "default_endpoint")]
    pub endpoint: Url,
    /// API key (Google AI Studio key).
    pub api_key: SecretRef,
    /// Live model id (`gemini-2.0-flash-live-001`).
    #[serde(default = "default_model")]
    pub model: String,
    /// Default voice (one of [`crate::caps::GEMINI_LIVE_VOICES`]).
    #[serde(default = "default_voice")]
    pub default_voice: String,
    /// Optional system instruction seeded on connect.
    #[serde(default)]
    pub instructions: Option<String>,
    /// Response modalities (`AUDIO`, `TEXT`).
    #[serde(default = "default_modalities")]
    pub response_modalities: Vec<String>,
    #[serde(default)]
    pub timeouts: Timeouts,
}

fn default_endpoint() -> Url {
    Url::parse(GEMINI_LIVE_WS_BASE).expect("GEMINI_LIVE_WS_BASE")
}
fn default_model() -> String {
    "models/gemini-2.0-flash-live-001".to_string()
}
fn default_voice() -> String {
    "Puck".to_string()
}
fn default_modalities() -> Vec<String> {
    vec!["AUDIO".to_string()]
}

impl GeminiLiveConfig {
    pub fn from_env() -> Self {
        Self {
            endpoint: default_endpoint(),
            api_key: SecretRef::env("GOOGLE_API_KEY"),
            model: default_model(),
            default_voice: default_voice(),
            instructions: None,
            response_modalities: default_modalities(),
            timeouts: Timeouts::default(),
        }
    }
}
