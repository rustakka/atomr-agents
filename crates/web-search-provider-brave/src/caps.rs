//! Static metadata about the Brave provider.

pub const PROVIDER_NAME: &str = "brave";

/// Brave's `count` parameter is capped at 20 for the free tier; the
/// paid Web Search API allows 50. We pick 20 as the conservative cap;
/// callers that have a higher quota can override the runtime's
/// behaviour via [`crate::config::BraveConfig`] in the future.
pub const MAX_RESULTS: u32 = 20;

/// Brave does not surface allow/deny lists, so callers post-filter.
pub const SUPPORTS_DOMAIN_FILTER: bool = false;

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
