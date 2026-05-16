//! Tavily provider config. Field layout mirrors the `stt-runtime-*`
//! configs so operators have one mental model across atomr-agents
//! remote backends.

use atomr_agents_stt_remote_core::{RateLimits, RetryPolicy, SecretRef, Timeouts};
use serde::{Deserialize, Serialize};
use url::Url;

pub const TAVILY_ENDPOINT: &str = "https://api.tavily.com/search";

/// Tavily `search_depth` parameter — `basic` is fast/cheap,
/// `advanced` is more thorough and costs more credits.
pub const DEFAULT_SEARCH_DEPTH: &str = "basic";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TavilyConfig {
    /// Search endpoint. Defaults to [`TAVILY_ENDPOINT`]. Override only
    /// for a relay.
    #[serde(default = "default_endpoint")]
    pub endpoint: Url,
    /// Tavily API key. Goes in the JSON body, not a header.
    pub api_key: SecretRef,
    /// `basic` (default) or `advanced`.
    #[serde(default = "default_search_depth")]
    pub default_search_depth: String,
    /// Whether to ask Tavily to include a synthesized answer alongside
    /// the hit list. We don't surface the answer through `WebSearchHit`
    /// today — flip on if a caller wants the raw provider response.
    #[serde(default)]
    pub include_answer: bool,
    #[serde(default)]
    pub timeouts: Timeouts,
    #[serde(default)]
    pub retry: RetryPolicy,
    #[serde(default)]
    pub rate_limits: RateLimits,
}

fn default_endpoint() -> Url {
    Url::parse(TAVILY_ENDPOINT).expect("TAVILY_ENDPOINT is a valid URL")
}

fn default_search_depth() -> String {
    DEFAULT_SEARCH_DEPTH.to_string()
}

impl TavilyConfig {
    /// Build a config that authenticates via `TAVILY_API_KEY`.
    pub fn from_env() -> Self {
        Self {
            endpoint: default_endpoint(),
            api_key: SecretRef::env("TAVILY_API_KEY"),
            default_search_depth: default_search_depth(),
            include_answer: false,
            timeouts: Timeouts::default(),
            retry: RetryPolicy::default(),
            rate_limits: RateLimits::default(),
        }
    }
}
