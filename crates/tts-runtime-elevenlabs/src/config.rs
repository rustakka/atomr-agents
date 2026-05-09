use atomr_agents_stt_remote_core::{RateLimits, RetryPolicy, SecretRef, Timeouts};
use serde::{Deserialize, Serialize};
use url::Url;

pub const ELEVENLABS_REST_BASE: &str = "https://api.elevenlabs.io/v1/";
pub const ELEVENLABS_WS_BASE: &str = "wss://api.elevenlabs.io/v1/text-to-speech/";
pub const ELEVENLABS_CONVAI_BASE: &str =
    "wss://api.elevenlabs.io/v1/convai/conversation";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElevenLabsConfig {
    #[serde(default = "default_rest")]
    pub rest_endpoint: Url,
    #[serde(default = "default_ws")]
    pub ws_endpoint: Url,
    /// Optional Conversational AI WS for `open_realtime`.
    #[serde(default = "default_convai")]
    pub convai_endpoint: Url,
    pub api_key: SecretRef,
    /// Default model. `eleven_turbo_v2_5` (fastest), `eleven_multilingual_v2`,
    /// `eleven_v3` (most expressive).
    #[serde(default = "default_model")]
    pub default_model: String,
    /// Default voice ID when [`atomr_agents_tts_core::VoiceRef`] is
    /// `Library` without an override.
    #[serde(default = "default_voice")]
    pub default_voice: String,
    /// Output format string (e.g. `"mp3_44100_128"`, `"pcm_24000"`,
    /// `"ulaw_8000"`).
    #[serde(default = "default_format")]
    pub default_output_format: String,
    /// `agent_id` for Conversational AI. Required for `open_realtime`.
    #[serde(default)]
    pub convai_agent_id: Option<String>,
    #[serde(default)]
    pub timeouts: Timeouts,
    #[serde(default)]
    pub retry: RetryPolicy,
    #[serde(default)]
    pub rate_limits: RateLimits,
}

fn default_rest() -> Url { Url::parse(ELEVENLABS_REST_BASE).expect("ELEVENLABS_REST_BASE") }
fn default_ws() -> Url { Url::parse(ELEVENLABS_WS_BASE).expect("ELEVENLABS_WS_BASE") }
fn default_convai() -> Url { Url::parse(ELEVENLABS_CONVAI_BASE).expect("ELEVENLABS_CONVAI_BASE") }
fn default_model() -> String { "eleven_turbo_v2_5".to_string() }
fn default_voice() -> String { "21m00Tcm4TlvDq8ikWAM".to_string() } // Rachel
fn default_format() -> String { "mp3_44100_128".to_string() }

impl ElevenLabsConfig {
    pub fn from_env() -> Self {
        Self {
            rest_endpoint: default_rest(),
            ws_endpoint: default_ws(),
            convai_endpoint: default_convai(),
            api_key: SecretRef::env("ELEVENLABS_API_KEY"),
            default_model: default_model(),
            default_voice: default_voice(),
            default_output_format: default_format(),
            convai_agent_id: std::env::var("ELEVENLABS_AGENT_ID").ok(),
            timeouts: Timeouts::default(),
            retry: RetryPolicy::default(),
            rate_limits: RateLimits::default(),
        }
    }
}
