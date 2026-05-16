//! Configuration for [`SignalProvider`](crate::SignalProvider).
//!
//! `SignalConfig` is parsed from the `config` JSON value on a
//! [`ChannelSpec`](atomr_agents_channel_core::ChannelSpec) — typically
//! the JSON looks like:
//!
//! ```json
//! {
//!   "endpoint": { "transport": "tcp", "address": "127.0.0.1:7583" },
//!   "account": "+15551234567",
//!   "default_channel_id": "channel-signal-demo"
//! }
//! ```
//!
//! Unix sockets are also supported:
//!
//! ```json
//! {
//!   "endpoint": { "transport": "unix", "address": "/tmp/signal-cli.sock" },
//!   "account": "+15551234567",
//!   "default_channel_id": "channel-signal-demo"
//! }
//! ```

use atomr_agents_channel_core::{ChannelError, ChannelId, Result};
use serde::{Deserialize, Serialize};

/// Transport used to reach the local `signal-cli` JSON-RPC daemon.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "transport", content = "address", rename_all = "snake_case")]
pub enum SignalEndpoint {
    /// TCP host:port, e.g. `127.0.0.1:7583`.
    Tcp(String),
    /// Unix socket path, e.g. `/tmp/signal-cli.sock`.
    Unix(String),
}

impl SignalEndpoint {
    /// Convenience: the textual address (host:port or path).
    pub fn address(&self) -> &str {
        match self {
            Self::Tcp(a) | Self::Unix(a) => a,
        }
    }
}

/// Parsed Signal provider config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalConfig {
    /// Where to reach the `signal-cli` daemon.
    pub endpoint: SignalEndpoint,
    /// The Signal account (E.164 phone number) this provider sends as
    /// and listens on. Forwarded as `params.account` to JSON-RPC calls
    /// so that multi-account daemons pick the right linked device.
    pub account: String,
    /// Channel id assigned to inbound JSON-RPC events (signal-cli
    /// notifications carry no channel context of their own).
    pub default_channel_id: ChannelId,
}

impl SignalConfig {
    /// Parse the JSON shape documented above.
    pub fn from_value(value: serde_json::Value) -> Result<Self> {
        serde_json::from_value(value)
            .map_err(|e| ChannelError::Config(format!("signal config: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_tcp_endpoint() {
        let cfg = SignalConfig::from_value(json!({
            "endpoint": {"transport": "tcp", "address": "127.0.0.1:7583"},
            "account": "+15551234567",
            "default_channel_id": "signal:demo"
        }))
        .unwrap();
        assert_eq!(cfg.endpoint, SignalEndpoint::Tcp("127.0.0.1:7583".into()));
        assert_eq!(cfg.account, "+15551234567");
        assert_eq!(cfg.default_channel_id.as_str(), "signal:demo");
    }

    #[test]
    fn parses_unix_endpoint() {
        let cfg = SignalConfig::from_value(json!({
            "endpoint": {"transport": "unix", "address": "/tmp/signal-cli.sock"},
            "account": "+15551234567",
            "default_channel_id": "signal:demo"
        }))
        .unwrap();
        assert_eq!(
            cfg.endpoint,
            SignalEndpoint::Unix("/tmp/signal-cli.sock".into())
        );
        assert_eq!(cfg.endpoint.address(), "/tmp/signal-cli.sock");
    }

    #[test]
    fn rejects_missing_fields() {
        let err = SignalConfig::from_value(json!({})).unwrap_err();
        assert!(matches!(err, ChannelError::Config(_)));
    }
}
