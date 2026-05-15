//! Long-lived interactive sessions.
//!
//! One `InteractiveSessionHandle` corresponds to a tmux session that
//! wraps a single CLI process; the harness fans terminal bytes out
//! through a broadcast channel and ingests keystrokes / resizes via
//! mpsc.

use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::{broadcast, mpsc};

use atomr_agents_coding_cli_core::{CliRequest, CliSessionId, CliVendorKind};

/// Frame the harness shows to clients (over WebSocket in the web companion).
#[derive(Debug, Clone)]
pub enum SessionEvent {
    /// Raw PTY bytes (UTF-8 or 8-bit safe — the client renders as ANSI).
    Bytes(Vec<u8>),
    /// Process exited.
    Exited { code: Option<i32> },
}

/// Frames clients send back.
#[derive(Debug, Clone)]
pub enum SessionTransport {
    Stdin(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Detach,
}

pub struct InteractiveSessionHandle {
    pub id: CliSessionId,
    pub vendor: CliVendorKind,
    pub tmux_session: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub request: CliRequest,

    /// Subscribe for PTY byte broadcast.
    pub events: broadcast::Sender<SessionEvent>,
    /// Send keystrokes / resizes / detach into the session.
    pub input: mpsc::Sender<SessionTransport>,

    /// Whether the session task has signalled completion. Kept here
    /// so the registry can prune cleanly.
    pub closed: Arc<parking_lot::Mutex<bool>>,
}

impl InteractiveSessionHandle {
    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    pub async fn send_stdin(&self, bytes: Vec<u8>) -> bool {
        self.input.send(SessionTransport::Stdin(bytes)).await.is_ok()
    }

    pub async fn resize(&self, cols: u16, rows: u16) -> bool {
        self.input
            .send(SessionTransport::Resize { cols, rows })
            .await
            .is_ok()
    }

    pub async fn detach(&self) -> bool {
        self.input.send(SessionTransport::Detach).await.is_ok()
    }
}

/// Concurrent registry of active sessions — shared by the harness and
/// the web companion.
#[derive(Default, Clone)]
pub struct SessionRegistry {
    inner: Arc<RwLock<Vec<Arc<InteractiveSessionHandle>>>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, h: Arc<InteractiveSessionHandle>) {
        self.inner.write().push(h);
    }

    pub fn get(&self, id: &CliSessionId) -> Option<Arc<InteractiveSessionHandle>> {
        self.inner.read().iter().find(|s| &s.id == id).cloned()
    }

    pub fn list(&self) -> Vec<Arc<InteractiveSessionHandle>> {
        self.inner.read().clone()
    }

    pub fn remove(&self, id: &CliSessionId) {
        self.inner.write().retain(|s| &s.id != id);
    }

    pub fn len(&self) -> usize {
        self.inner.read().len()
    }
    pub fn is_empty(&self) -> bool {
        self.inner.read().is_empty()
    }
}
