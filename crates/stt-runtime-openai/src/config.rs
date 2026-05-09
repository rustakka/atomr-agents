//! OpenAI STT config. Mirrors the field layout of
//! `atomr-infer/inference-runtime-openai`'s config so operators have
//! one mental model for "this is an OpenAI deployment".

use atomr_agents_stt_remote_core::{RateLimits, RetryPolicy, SecretRef, Timeouts};
use serde::{Deserialize, Serialize};
use url::Url;

pub const OPENAI_BASE_URL: &str = "https://api.openai.com/v1/";
pub const DEFAULT_MODEL: &str = "whisper-1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiSttConfig {
    /// Base URL. Defaults to `https://api.openai.com/v1/`. Override
    /// for Azure OpenAI or a relay.
    #[serde(default = "default_endpoint")]
    pub endpoint: Url,
    pub api_key: SecretRef,
    /// Optional `OpenAI-Organization` header.
    #[serde(default)]
    pub organization: Option<String>,
    /// Default model when [`crate::TranscribeOptions::model`] is `None`.
    /// `"whisper-1"` (default) or `"gpt-4o-transcribe"` /
    /// `"gpt-4o-mini-transcribe"`.
    #[serde(default = "default_model")]
    pub default_model: String,
    /// Default BCP-47 language hint. `None` triggers detection.
    #[serde(default)]
    pub default_language: Option<String>,
    #[serde(default)]
    pub timeouts: Timeouts,
    #[serde(default)]
    pub retry: RetryPolicy,
    #[serde(default)]
    pub rate_limits: RateLimits,
}

fn default_endpoint() -> Url {
    Url::parse(OPENAI_BASE_URL).expect("OPENAI_BASE_URL is a valid URL")
}

fn default_model() -> String {
    DEFAULT_MODEL.to_string()
}

impl OpenAiSttConfig {
    /// Build a config that authenticates via `OPENAI_API_KEY`. Use
    /// for the common case.
    pub fn from_env() -> Self {
        Self {
            endpoint: default_endpoint(),
            api_key: SecretRef::env("OPENAI_API_KEY"),
            organization: None,
            default_model: default_model(),
            default_language: None,
            timeouts: Timeouts::default(),
            retry: RetryPolicy::default(),
            rate_limits: RateLimits::default(),
        }
    }
}
