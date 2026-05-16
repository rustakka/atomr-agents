//! Static metadata about the Tavily provider.

/// Provider id reported by [`atomr_agents_web_search_core::WebSearch::provider_name`].
pub const PROVIDER_NAME: &str = "tavily";

/// Tavily caps `max_results` at 20 per request as of late 2024.
pub const MAX_RESULTS: u32 = 20;

/// Whether the provider supports `include_domains` / `exclude_domains`
/// server-side. Tavily does.
pub const SUPPORTS_DOMAIN_FILTER: bool = true;

/// Static capability bundle (kept simple — providers may grow into a
/// struct mirroring `stt-core::Capabilities` if more callers need it).
#[derive(Debug, Clone, Copy)]
pub struct Caps {
    pub provider_name: &'static str,
    pub max_results: u32,
    pub supports_domain_filter: bool,
}

pub const CAPS: Caps = Caps {
    provider_name: PROVIDER_NAME,
    max_results: MAX_RESULTS,
    supports_domain_filter: SUPPORTS_DOMAIN_FILTER,
};
