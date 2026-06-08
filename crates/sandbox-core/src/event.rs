//! Sandbox lifecycle events broadcast by the harness — the richer,
//! sandbox-specific complement to the generic `Event::ToolInvoked` the tool
//! emits on the process-local `EventBus`.
//!
//! Tagged enum (`{"kind": "...", ...}`) so web/SSE clients can switch on
//! `kind`, matching `CodingCliEvent`.

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::id::{ExecId, SandboxId, SnapshotId};
use crate::profile::{Language, SandboxProfile};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SandboxEvent {
    Created {
        id: SandboxId,
        profile: SandboxProfile,
        boot_ms: u64,
    },
    ExecStarted {
        id: SandboxId,
        language: Language,
    },
    ExecEnded {
        id: SandboxId,
        exec_id: ExecId,
        exit_code: Option<i32>,
        elapsed_ms: u64,
    },
    ExecError {
        id: SandboxId,
        error: String,
    },
    Forked {
        parent: SandboxId,
        child: SandboxId,
        snapshot: SnapshotId,
    },
    Destroyed {
        id: SandboxId,
    },
}

/// Subscriber handle backed by a `broadcast::Receiver`. Drops missed events
/// silently — same semantics as `CodingCliEventStream`.
pub struct SandboxEventStream {
    rx: broadcast::Receiver<SandboxEvent>,
}

impl SandboxEventStream {
    pub fn new(rx: broadcast::Receiver<SandboxEvent>) -> Self {
        Self { rx }
    }

    /// Wait for the next event. Returns `None` once the channel closes.
    pub async fn recv(&mut self) -> Option<SandboxEvent> {
        loop {
            match self.rx.recv().await {
                Ok(ev) => return Some(ev),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}
