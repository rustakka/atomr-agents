//! Brave Search `WebSearch` provider —
//! `GET https://api.search.brave.com/res/v1/web/search`.
//!
//! Auth via the `X-Subscription-Token` header. Recency requests are
//! translated to Brave's `freshness=pd|pw|pm|py` knob.

#![forbid(unsafe_code)]

mod caps;
mod config;
mod http;
mod runner;
mod wire;

pub use caps::CAPS;
pub use config::{BraveConfig, BRAVE_ENDPOINT};
pub use runner::BraveWebSearch;
