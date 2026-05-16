//! UDP Live Link sink configuration.

use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

use atomr_agents_avatar_core::AvatarError;

/// Configuration for [`crate::LiveLinkSink`].
///
/// The default target (`127.0.0.1:6666`) is what the companion UE5
/// receiver plugin listens on out-of-the-box; production deployments
/// override `addr` to point at the workstation running Unreal.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LiveLinkConfig {
    /// UDP `host:port` to send framed datagrams to.
    pub addr: SocketAddr,
    /// Local UDP `host:port` to bind the sender to. `None` ⇒
    /// ephemeral on `0.0.0.0:0`.
    #[serde(default)]
    pub bind: Option<SocketAddr>,
    /// Soft cap on emitter throughput, in frames per second. The
    /// harness pre-paces frames upstream; this is a defensive floor.
    /// Set to `0` to disable.
    #[serde(default = "default_max_fps")]
    pub max_fps: u32,
    /// Logical name for this sink (logs / telemetry).
    #[serde(default = "default_label")]
    pub label: String,
}

fn default_max_fps() -> u32 {
    60
}

fn default_label() -> String {
    "livelink-udp".to_string()
}

impl LiveLinkConfig {
    /// Sensible local dev default: `127.0.0.1:6666`, max 60 fps.
    pub fn loopback() -> Self {
        Self {
            addr: "127.0.0.1:6666".parse().expect("static addr parses"),
            bind: None,
            max_fps: default_max_fps(),
            label: default_label(),
        }
    }

    /// Build from a JSON value (used by the Python facade + registry).
    pub fn from_value(value: serde_json::Value) -> Result<Self, AvatarError> {
        serde_json::from_value(value).map_err(|e| AvatarError::config(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_default_parses() {
        let cfg = LiveLinkConfig::loopback();
        assert_eq!(cfg.addr.port(), 6666);
        assert_eq!(cfg.max_fps, 60);
    }

    #[test]
    fn from_json_minimal() {
        let cfg = LiveLinkConfig::from_value(serde_json::json!({
            "addr": "10.0.0.42:6666"
        }))
        .unwrap();
        assert_eq!(cfg.addr.ip().to_string(), "10.0.0.42");
        assert_eq!(cfg.max_fps, 60);
        assert_eq!(cfg.label, "livelink-udp");
    }
}
