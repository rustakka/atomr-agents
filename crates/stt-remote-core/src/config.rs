//! Config shapes shared across all cloud STT backends.
//!
//! These mirror the field layout of `atomr-infer`'s remote configs
//! so operators have one mental model. They're intentionally
//! re-implemented (not re-exported) to keep `stt-remote-core` free
//! of the heavier `inference-remote-core` deps (hyper, tower,
//! distributed-data CRDT).

use std::env;
use std::path::PathBuf;
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use atomr_agents_stt_core::SttError;

/// Reference to a secret. Resolves at use time, not at config load,
/// so unset env vars surface as errors at the call site instead of
/// during deserialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "from", rename_all = "snake_case")]
pub enum SecretRef {
    /// Inline literal (test fixtures, ephemeral CI secrets).
    Literal { value: String },
    /// `${VAR}` from process env.
    Env { name: String },
    /// First non-empty line of the named file.
    File { path: PathBuf },
}

impl SecretRef {
    pub fn literal(s: impl Into<String>) -> Self {
        Self::Literal { value: s.into() }
    }

    pub fn env(name: impl Into<String>) -> Self {
        Self::Env { name: name.into() }
    }

    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self::File { path: path.into() }
    }

    /// Resolve to a `SecretString`. Errors are mapped to
    /// `SttError::Auth`, never logged with the value.
    pub fn resolve(&self) -> Result<SecretString, SttError> {
        match self {
            SecretRef::Literal { value } => Ok(SecretString::new(value.clone().into())),
            SecretRef::Env { name } => env::var(name)
                .map(|v| SecretString::new(v.into()))
                .map_err(|_| SttError::Auth),
            SecretRef::File { path } => {
                let bytes = std::fs::read_to_string(path).map_err(|_| SttError::Auth)?;
                let line = bytes
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .ok_or(SttError::Auth)?
                    .trim()
                    .to_string();
                Ok(SecretString::new(line.into()))
            }
        }
    }

    /// Convenience: produce the secret as `Authorization: Bearer …`.
    /// The returned `SecretString` keeps the value zeroized on drop.
    pub fn bearer(&self) -> Result<SecretString, SttError> {
        let s = self.resolve()?;
        Ok(SecretString::new(
            format!("Bearer {}", s.expose_secret()).into(),
        ))
    }
}

/// Per-call timeout knobs. All optional; backends pick sensible
/// defaults when fields are `None`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timeouts {
    /// Total wall-clock cap for the entire request (seconds).
    pub total_secs: Option<u64>,
    /// TCP-connect timeout (seconds).
    pub connect_secs: Option<u64>,
    /// Per-read timeout for streaming responses (seconds).
    pub read_secs: Option<u64>,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            total_secs: Some(120),
            connect_secs: Some(10),
            read_secs: Some(60),
        }
    }
}

impl Timeouts {
    pub fn total(&self) -> Option<Duration> {
        self.total_secs.map(Duration::from_secs)
    }
    pub fn connect(&self) -> Option<Duration> {
        self.connect_secs.map(Duration::from_secs)
    }
    pub fn read(&self) -> Option<Duration> {
        self.read_secs.map(Duration::from_secs)
    }
}

/// Exponential-backoff retry policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
    pub multiplier: f32,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff_ms: 250,
            max_backoff_ms: 8_000,
            multiplier: 2.0,
        }
    }
}

impl RetryPolicy {
    pub fn backoff_for(&self, attempt: u32) -> Duration {
        let base = (self.initial_backoff_ms as f64)
            * (self.multiplier as f64).powi(attempt.saturating_sub(1) as i32);
        let capped = base.min(self.max_backoff_ms as f64);
        Duration::from_millis(capped as u64)
    }
}

/// Provider-advertised rate limits. Surfaced from
/// [`crate::client::classify_status`] when a `429` carries a
/// `Retry-After` header so callers can either pause or surface a
/// typed `SttError::RateLimited`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RateLimits {
    pub requests_per_minute: Option<u32>,
    pub tokens_per_minute: Option<u32>,
    pub concurrent_streams: Option<u16>,
}
