//! SerpAPI `WebSearch` provider — `GET https://serpapi.com/search`.
//!
//! Auth via `api_key` query parameter. Defaults to `engine=google`.
//! Translates [`atomr_agents_web_search_core::WebSearchRequest::recency_days`]
//! into Google's `tbs=qdr:d|w|m|y` knob.

#![forbid(unsafe_code)]

mod caps;
mod config;
mod http;
mod runner;
mod wire;

pub use caps::CAPS;
pub use config::{SerpApiConfig, SERPAPI_ENDPOINT};
pub use runner::SerpApiWebSearch;
