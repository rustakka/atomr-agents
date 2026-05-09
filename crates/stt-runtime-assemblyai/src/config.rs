use atomr_agents_stt_remote_core::{RateLimits, RetryPolicy, SecretRef, Timeouts};
use serde::{Deserialize, Serialize};
use url::Url;

pub const ASSEMBLY_REST_BASE: &str = "https://api.assemblyai.com/v2/";
pub const ASSEMBLY_WS_BASE: &str = "wss://streaming.assemblyai.com/v3/ws";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssemblyAiConfig {
    #[serde(default = "default_rest")]
    pub rest_endpoint: Url,
    #[serde(default = "default_ws")]
    pub ws_endpoint: Url,
    pub api_key: SecretRef,
    /// Default speech model (e.g. `"universal"`, `"slam-1"`).
    #[serde(default = "default_model")]
    pub default_model: String,
    #[serde(default)]
    pub default_language: Option<String>,
    #[serde(default = "default_speaker_labels")]
    pub default_speaker_labels: bool,
    #[serde(default)]
    pub timeouts: Timeouts,
    #[serde(default)]
    pub retry: RetryPolicy,
    #[serde(default)]
    pub rate_limits: RateLimits,
}

fn default_rest() -> Url {
    Url::parse(ASSEMBLY_REST_BASE).expect("ASSEMBLY_REST_BASE")
}
fn default_ws() -> Url {
    Url::parse(ASSEMBLY_WS_BASE).expect("ASSEMBLY_WS_BASE")
}
fn default_model() -> String {
    "universal".into()
}
fn default_speaker_labels() -> bool {
    false
}

impl AssemblyAiConfig {
    pub fn from_env() -> Self {
        Self {
            rest_endpoint: default_rest(),
            ws_endpoint: default_ws(),
            api_key: SecretRef::env("ASSEMBLYAI_API_KEY"),
            default_model: default_model(),
            default_language: None,
            default_speaker_labels: false,
            timeouts: Timeouts::default(),
            retry: RetryPolicy::default(),
            rate_limits: RateLimits::default(),
        }
    }
}
