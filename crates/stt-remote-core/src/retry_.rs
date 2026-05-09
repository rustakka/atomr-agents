//! Generic exponential-backoff retry helper. Backends pass a closure
//! that returns `Result<T, SttError>` and a [`RetryPolicy`]; this
//! helper retries only on errors classified as transient
//! (transport, 5xx, rate-limit) — auth/decode/etc are surfaced
//! immediately.

use std::future::Future;

use atomr_agents_stt_core::{Result, SttError};
use tokio::time::sleep;

use crate::config::RetryPolicy;

fn is_transient(e: &SttError) -> bool {
    matches!(
        e,
        SttError::Transport(_)
            | SttError::RateLimited { .. }
            | SttError::Backend { status: 500..=599, .. }
    )
}

/// Run `op` up to `policy.max_attempts` times with exponential
/// backoff between transient failures. The returned `Result`
/// preserves the last error.
pub async fn retry<T, F, Fut>(policy: &RetryPolicy, mut op: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut last: Option<SttError> = None;
    for attempt in 1..=policy.max_attempts {
        match op().await {
            Ok(t) => return Ok(t),
            Err(e) => {
                if !is_transient(&e) || attempt == policy.max_attempts {
                    return Err(e);
                }
                let pause = match &e {
                    SttError::RateLimited { retry_after_ms } => {
                        std::time::Duration::from_millis(*retry_after_ms)
                    }
                    _ => policy.backoff_for(attempt),
                };
                tracing::debug!(
                    attempt,
                    backoff_ms = pause.as_millis() as u64,
                    error = %e,
                    "stt retry: transient error, backing off",
                );
                last = Some(e);
                sleep(pause).await;
            }
        }
    }
    Err(last.unwrap_or_else(|| SttError::internal("retry: exhausted with no last error")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn retries_then_succeeds() {
        let policy = RetryPolicy {
            max_attempts: 3,
            initial_backoff_ms: 1,
            max_backoff_ms: 1,
            multiplier: 1.0,
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let calls2 = calls.clone();
        let r: Result<u32> = retry(&policy, move || {
            let calls = calls2.clone();
            async move {
                let n = calls.fetch_add(1, Ordering::SeqCst);
                if n < 2 {
                    Err(SttError::transport("flaky"))
                } else {
                    Ok(42)
                }
            }
        })
        .await;
        assert_eq!(r.unwrap(), 42);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn does_not_retry_auth() {
        let policy = RetryPolicy::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let calls2 = calls.clone();
        let r: Result<()> = retry(&policy, move || {
            let calls = calls2.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(SttError::Auth)
            }
        })
        .await;
        assert!(matches!(r, Err(SttError::Auth)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
