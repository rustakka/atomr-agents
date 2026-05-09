use atomr_agents_stt_remote_core::{RateLimits, RetryPolicy, SecretRef, Timeouts};
use serde::{Deserialize, Serialize};
use url::Url;

pub const OPENAI_BASE_URL: &str = "https://api.openai.com/v1/";
pub const DEFAULT_MODEL: &str = "tts-1";
pub const DEFAULT_VOICE: &str = "alloy";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiTtsConfig {
    #[serde(default = "default_endpoint")]
    pub endpoint: Url,
    pub api_key: SecretRef,
    #[serde(default)]
    pub organization: Option<String>,
    /// Default model. `tts-1` (fastest), `tts-1-hd` (higher fidelity),
    /// or `gpt-4o-mini-tts` (steerable via `instructions`).
    #[serde(default = "default_model")]
    pub default_model: String,
    /// Default voice when [`atomr_agents_tts_core::VoiceRef::Library`]
    /// is used without overriding. Falls back to `alloy`.
    #[serde(default = "default_voice")]
    pub default_voice: String,
    /// Output container preference (`mp3`, `opus`, `aac`, `flac`,
    /// `wav`, `pcm`). `mp3` is the OpenAI default.
    #[serde(default = "default_format")]
    pub default_format: String,
    /// Speed multiplier (0.25 – 4.0). 1.0 = unchanged.
    #[serde(default)]
    pub default_speed: Option<f32>,
    #[serde(default)]
    pub timeouts: Timeouts,
    #[serde(default)]
    pub retry: RetryPolicy,
    #[serde(default)]
    pub rate_limits: RateLimits,
}

fn default_endpoint() -> Url { Url::parse(OPENAI_BASE_URL).expect("OPENAI_BASE_URL") }
fn default_model() -> String { DEFAULT_MODEL.to_string() }
fn default_voice() -> String { DEFAULT_VOICE.to_string() }
fn default_format() -> String { "mp3".to_string() }

impl OpenAiTtsConfig {
    pub fn from_env() -> Self {
        Self {
            endpoint: default_endpoint(),
            api_key: SecretRef::env("OPENAI_API_KEY"),
            organization: None,
            default_model: default_model(),
            default_voice: default_voice(),
            default_format: default_format(),
            default_speed: None,
            timeouts: Timeouts::default(),
            retry: RetryPolicy::default(),
            rate_limits: RateLimits::default(),
        }
    }
}
