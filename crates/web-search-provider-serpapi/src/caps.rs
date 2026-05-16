//! Static metadata about the SerpAPI provider.

pub const PROVIDER_NAME: &str = "serpapi";

/// SerpAPI caps Google's `num` parameter at 100.
pub const MAX_RESULTS: u32 = 100;

/// SerpAPI does not directly expose domain allow/deny; we post-filter
/// instead.
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
