//! Tavily `WebSearch` provider — `POST https://api.tavily.com/search`.
//!
//! Auth via API key in the JSON body. Maps Tavily's cleaned-text
//! `content` field into [`atomr_agents_web_search_core::WebSearchHit::content`].

#![forbid(unsafe_code)]

mod caps;
mod config;
mod http;
mod runner;
mod wire;

pub use caps::CAPS;
pub use config::{TavilyConfig, TAVILY_ENDPOINT};
pub use runner::TavilyWebSearch;
