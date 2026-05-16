//! SerpAPI provider config.

use atomr_agents_stt_remote_core::{RateLimits, RetryPolicy, SecretRef, Timeouts};
use serde::{Deserialize, Serialize};
use url::Url;

pub const SERPAPI_ENDPOINT: &str = "https://serpapi.com/search";

/// Google is SerpAPI's default engine. Other engines (`bing`,
/// `duckduckgo`, …) are also accepted by the API but only `google` is
/// validated against our `WebSearchHit` mapping today.
pub const DEFAULT_ENGINE: &str = "google";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerpApiConfig {
    /// Search endpoint. Defaults to [`SERPAPI_ENDPOINT`]. Override for
    /// the dedicated regional endpoints (`*.serpapi.com`) or a relay.
    #[serde(default = "default_endpoint")]
    pub endpoint: Url,
    /// SerpAPI key. Goes in the `api_key` query parameter.
    pub api_key: SecretRef,
    /// Search engine — `google` (default), `bing`, `duckduckgo`, etc.
    #[serde(default = "default_engine")]
    pub engine: String,
    #[serde(default)]
    pub timeouts: Timeouts,
    #[serde(default)]
    pub retry: RetryPolicy,
    #[serde(default)]
    pub rate_limits: RateLimits,
}

fn default_endpoint() -> Url {
    Url::parse(SERPAPI_ENDPOINT).expect("SERPAPI_ENDPOINT is a valid URL")
}

fn default_engine() -> String {
    DEFAULT_ENGINE.to_string()
}

impl SerpApiConfig {
    /// Build a config that authenticates via `SERPAPI_KEY`.
    pub fn from_env() -> Self {
        Self {
            endpoint: default_endpoint(),
            api_key: SecretRef::env("SERPAPI_KEY"),
            engine: default_engine(),
            timeouts: Timeouts::default(),
            retry: RetryPolicy::default(),
            rate_limits: RateLimits::default(),
        }
    }
}
