use atomr_agents_stt_remote_core::{RateLimits, RetryPolicy, SecretRef, Timeouts};
use serde::{Deserialize, Serialize};
use url::Url;

pub const DEEPGRAM_REST_BASE: &str = "https://api.deepgram.com/v1/";
pub const DEEPGRAM_WS_BASE: &str = "wss://api.deepgram.com/v1/listen";
pub const DEFAULT_MODEL: &str = "nova-3";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepgramConfig {
    #[serde(default = "default_rest_endpoint")]
    pub rest_endpoint: Url,
    #[serde(default = "default_ws_endpoint")]
    pub ws_endpoint: Url,
    pub api_key: SecretRef,
    #[serde(default = "default_model")]
    pub default_model: String,
    #[serde(default)]
    pub default_language: Option<String>,
    /// Tier override (e.g. `"enhanced"`, `"base"`). Most users let
    /// the model choose.
    #[serde(default)]
    pub default_tier: Option<String>,
    #[serde(default)]
    pub timeouts: Timeouts,
    #[serde(default)]
    pub retry: RetryPolicy,
    #[serde(default)]
    pub rate_limits: RateLimits,
}

fn default_rest_endpoint() -> Url {
    Url::parse(DEEPGRAM_REST_BASE).expect("DEEPGRAM_REST_BASE")
}
fn default_ws_endpoint() -> Url {
    Url::parse(DEEPGRAM_WS_BASE).expect("DEEPGRAM_WS_BASE")
}
fn default_model() -> String {
    DEFAULT_MODEL.into()
}

impl DeepgramConfig {
    pub fn from_env() -> Self {
        Self {
            rest_endpoint: default_rest_endpoint(),
            ws_endpoint: default_ws_endpoint(),
            api_key: SecretRef::env("DEEPGRAM_API_KEY"),
            default_model: default_model(),
            default_language: None,
            default_tier: None,
            timeouts: Timeouts::default(),
            retry: RetryPolicy::default(),
            rate_limits: RateLimits::default(),
        }
    }
}
