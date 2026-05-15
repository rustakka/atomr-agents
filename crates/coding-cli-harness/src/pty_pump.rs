//! Pumps PTY bytes ↔ async channels for one interactive session.
//!
//! Spawned by `interactive::start_session`; not part of the public
//! crate surface.

use std::sync::Arc;

use tokio::sync::{broadcast, mpsc};
use tracing::warn;

use atomr_agents_coding_cli_isolator::ProcessHandle;

use crate::session::{SessionEvent, SessionTransport};

const PTY_BROADCAST_CAPACITY: usize = 512;
const PTY_INPUT_CAPACITY: usize = 64;

pub(crate) struct PumpHandles {
    pub events_tx: broadcast::Sender<SessionEvent>,
    pub input_tx: mpsc::Sender<SessionTransport>,
    pub closed: Arc<parking_lot::Mutex<bool>>,
}

pub(crate) fn spawn(mut handle: Box<dyn ProcessHandle>) -> PumpHandles {
    let (events_tx, _events_rx) = broadcast::channel(PTY_BROADCAST_CAPACITY);
    let (input_tx, mut input_rx) = mpsc::channel::<SessionTransport>(PTY_INPUT_CAPACITY);
    let closed = Arc::new(parking_lot::Mutex::new(false));

    let stdout_rx = handle.take_stdout();
    let stdin_tx = handle.take_stdin();

    // PTY → broadcast
    if let Some(mut rx) = stdout_rx {
        let events_tx = events_tx.clone();
        tokio::spawn(async move {
            while let Some(chunk) = rx.recv().await {
                if events_tx.send(SessionEvent::Bytes(chunk)).is_err() {
                    // no subscribers — keep draining so the PTY doesn't block.
                }
            }
        });
    }

    // input mpsc → PTY stdin + control
    let closed_for_task = closed.clone();
    let events_tx_for_exit = events_tx.clone();
    tokio::spawn(async move {
        let mut handle = handle; // moved into task to own resize/wait
        while let Some(frame) = input_rx.recv().await {
            match frame {
                SessionTransport::Stdin(bytes) => {
                    if let Some(tx) = &stdin_tx {
                        if tx.send(bytes).await.is_err() {
                            break;
                        }
                    }
                }
                SessionTransport::Resize { cols, rows } => {
                    if let Err(e) = handle.resize_pty(cols, rows).await {
                        warn!(error = %e, "pty resize failed");
                    }
                }
                SessionTransport::Detach => {
                    // Clients detaching is a UI-only signal; tmux is
                    // already daemonized, so we just stop forwarding
                    // input from this client. The session lives on.
                    break;
                }
            }
        }
        // Wait for child exit so we can announce.
        let status = handle.wait().await;
        *closed_for_task.lock() = true;
        let _ = events_tx_for_exit.send(SessionEvent::Exited {
            code: status.ok().and_then(|s| s.code),
        });
    });

    PumpHandles {
        events_tx,
        input_tx,
        closed,
    }
}
