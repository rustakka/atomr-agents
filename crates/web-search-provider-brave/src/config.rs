//! Brave provider config.

use atomr_agents_stt_remote_core::{RateLimits, RetryPolicy, SecretRef, Timeouts};
use serde::{Deserialize, Serialize};
use url::Url;

pub const BRAVE_ENDPOINT: &str = "https://api.search.brave.com/res/v1/web/search";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BraveConfig {
    /// Search endpoint. Defaults to [`BRAVE_ENDPOINT`].
    #[serde(default = "default_endpoint")]
    pub endpoint: Url,
    /// Brave subscription token. Sent as `X-Subscription-Token`.
    pub api_key: SecretRef,
    /// Optional country override (`US`, `GB`, `DE`, …). When `None`,
    /// the runtime falls back to `request.locale` (taking the
    /// upper-cased region segment after the `-`).
    #[serde(default)]
    pub default_country: Option<String>,
    #[serde(default)]
    pub timeouts: Timeouts,
    #[serde(default)]
    pub retry: RetryPolicy,
    #[serde(default)]
    pub rate_limits: RateLimits,
}

fn default_endpoint() -> Url {
    Url::parse(BRAVE_ENDPOINT).expect("BRAVE_ENDPOINT is a valid URL")
}

impl BraveConfig {
    /// Build a config that authenticates via `BRAVE_API_KEY`.
    pub fn from_env() -> Self {
        Self {
            endpoint: default_endpoint(),
            api_key: SecretRef::env("BRAVE_API_KEY"),
            default_country: None,
            timeouts: Timeouts::default(),
            retry: RetryPolicy::default(),
            rate_limits: RateLimits::default(),
        }
    }
}
