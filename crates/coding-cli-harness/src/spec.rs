use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Top-level harness configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodingCliHarnessSpec {
    /// Default model used when a `CliRequest` does not name one.
    #[serde(default)]
    pub default_model: Option<String>,
    /// Default wall-clock budget per headless run.
    #[serde(default = "default_wall_clock")]
    pub default_wall_clock: Duration,
    /// Cap on simultaneous headless runs. Beyond this the harness
    /// queues incoming requests (FIFO).
    #[serde(default = "default_max_runs")]
    pub max_concurrent_runs: usize,
    /// Cap on simultaneous interactive sessions.
    #[serde(default = "default_max_sessions")]
    pub max_concurrent_sessions: usize,
    /// Capacity of the broadcast event channel.
    #[serde(default = "default_channel_cap")]
    pub event_channel_capacity: usize,
    /// Optional root path where run logs and configs are kept.
    #[serde(default)]
    pub state_dir: Option<PathBuf>,
}

impl Default for CodingCliHarnessSpec {
    fn default() -> Self {
        Self {
            default_model: None,
            default_wall_clock: default_wall_clock(),
            max_concurrent_runs: default_max_runs(),
            max_concurrent_sessions: default_max_sessions(),
            event_channel_capacity: default_channel_cap(),
            state_dir: None,
        }
    }
}

fn default_wall_clock() -> Duration {
    Duration::from_secs(30 * 60)
}
fn default_max_runs() -> usize {
    8
}
fn default_max_sessions() -> usize {
    16
}
fn default_channel_cap() -> usize {
    512
}
