//! Provider-local HTTP helpers: status-code classification + retry
//! around a `reqwest::Client`. We reuse
//! [`atomr_agents_stt_remote_core::build_http_client`] for the client
//! itself (timeouts + TLS feature gating live there), and re-implement
//! the small amount of error-mapping logic locally so we surface
//! [`WebSearchError`] (not `SttError`).

use std::future::Future;
use std::time::Duration;

use atomr_agents_stt_remote_core::{RetryPolicy, Timeouts};
use atomr_agents_web_search_core::{Result, WebSearchError};
use reqwest::Client;
use tokio::time::sleep;

/// Wrap [`atomr_agents_stt_remote_core::build_http_client`], mapping
/// any builder error into [`WebSearchError::Config`].
pub(crate) fn build_http_client(timeouts: &Timeouts) -> Result<Client> {
    atomr_agents_stt_remote_core::build_http_client(timeouts)
        .map_err(|e| WebSearchError::Config(format!("build http client: {e}")))
}

/// Classify an HTTP status (plus optional Retry-After seconds) into a
/// typed [`WebSearchError`]. 4xx auth → `Config`; everything else
/// non-2xx → `Provider`.
pub(crate) fn classify_status(
    status: u16,
    retry_after_secs: Option<u64>,
    body_message: impl Into<String>,
) -> WebSearchError {
    let message = body_message.into();
    match status {
        401 | 403 => WebSearchError::Config(format!("auth rejected ({status}): {message}")),
        429 => WebSearchError::Provider(format!(
            "rate-limited; retry after {}s: {message}",
            retry_after_secs.unwrap_or(1)
        )),
        s @ 500..=599 => WebSearchError::Provider(format!("server error {s}: {message}")),
        s => WebSearchError::Provider(format!("provider returned {s}: {message}")),
    }
}

/// Conservative retry classifier — retry only transport failures and
/// 5xx / rate-limit responses. Auth / 4xx surface immediately.
fn is_transient(e: &WebSearchError) -> bool {
    match e {
        WebSearchError::Transport(_) => true,
        WebSearchError::Provider(msg) => msg.starts_with("rate-limited") || msg.starts_with("server error"),
        _ => false,
    }
}

/// Run `op` up to `policy.max_attempts` times with exponential
/// backoff between transient failures, mirroring
/// `atomr_agents_stt_remote_core::retry` but parameterised over
/// [`WebSearchError`].
pub(crate) async fn retry<T, F, Fut>(policy: &RetryPolicy, mut op: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut last: Option<WebSearchError> = None;
    for attempt in 1..=policy.max_attempts {
        match op().await {
            Ok(t) => return Ok(t),
            Err(e) => {
                if !is_transient(&e) || attempt == policy.max_attempts {
                    return Err(e);
                }
                let pause: Duration = policy.backoff_for(attempt);
                tracing::debug!(
                    attempt,
                    backoff_ms = pause.as_millis() as u64,
                    error = %e,
                    "web-search retry: transient error, backing off",
                );
                last = Some(e);
                sleep(pause).await;
            }
        }
    }
    Err(last.unwrap_or_else(|| WebSearchError::Other("retry: exhausted with no last error".into())))
}
