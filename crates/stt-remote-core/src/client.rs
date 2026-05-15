//! `reqwest::Client` builder + HTTP error classification.

use std::path::Path;

use atomr_agents_stt_core::{AudioFormat, SttError};
use reqwest::Client;

use crate::config::Timeouts;

/// Build a `reqwest::Client` with the configured timeouts and the
/// TLS backend chosen via Cargo features (rustls by default).
pub fn build_http_client(timeouts: &Timeouts) -> Result<Client, SttError> {
    let mut b = Client::builder().user_agent(concat!("atomr-agents-stt/", env!("CARGO_PKG_VERSION"),));
    if let Some(t) = timeouts.total() {
        b = b.timeout(t);
    }
    if let Some(t) = timeouts.connect() {
        b = b.connect_timeout(t);
    }
    if let Some(t) = timeouts.read() {
        b = b.read_timeout(t);
    }
    b.build()
        .map_err(|e| SttError::transport(format!("build_http_client: {e}")))
}

/// Map an HTTP status (plus an optional `Retry-After` header value
/// in seconds) to a typed [`SttError`]. Used by every backend's
/// non-2xx branch.
pub fn classify_status(
    status: u16,
    retry_after_secs: Option<u64>,
    body_message: impl Into<String>,
) -> SttError {
    let message = body_message.into();
    match status {
        401 | 403 => SttError::Auth,
        429 => SttError::RateLimited {
            retry_after_ms: retry_after_secs.unwrap_or(1) * 1_000,
        },
        s @ 500..=599 => SttError::Backend { status: s, message },
        s => SttError::Backend { status: s, message },
    }
}

/// Pick a sensible filename for a multipart upload from the input
/// path (if any) or the declared format.
pub fn multipart_filename_for(path: Option<&Path>, format: &AudioFormat) -> String {
    if let Some(p) = path {
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            return name.to_string();
        }
    }
    format!("audio.{}", format.extension())
}
