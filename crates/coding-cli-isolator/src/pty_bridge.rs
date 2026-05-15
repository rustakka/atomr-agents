//! Bridges a blocking `portable_pty::MasterPty` to async byte channels.
//!
//! `portable-pty` only exposes blocking `Read` / `Write` on the master
//! side. We spawn two `tokio::task::spawn_blocking` workers — one for
//! reads, one for writes — and trade bytes through bounded `mpsc`
//! channels with the async world.

use std::io::{Read, Write};
use std::sync::Arc;

use parking_lot::Mutex;
use portable_pty::{Child, MasterPty, PtySize};
use tokio::sync::mpsc;

use crate::error::IsolatorError;
use crate::handle::ExitStatus;

const READ_BUF: usize = 4 * 1024;
const CHANNEL_CAPACITY: usize = 128;

/// Set of channels + control handles produced by [`spawn_pty_bridge`].
pub(crate) struct PtyBridge {
    pub stdout_rx: Option<mpsc::Receiver<Vec<u8>>>,
    pub stdin_tx: Option<mpsc::Sender<Vec<u8>>>,
    pub master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    pub child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
}

pub(crate) fn spawn_pty_bridge(
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
) -> Result<PtyBridge, IsolatorError> {
    let master = Arc::new(Mutex::new(master));
    let child = Arc::new(Mutex::new(child));

    let reader = master
        .lock()
        .try_clone_reader()
        .map_err(|e| IsolatorError::Pty(format!("clone reader: {e}")))?;
    let writer = master
        .lock()
        .take_writer()
        .map_err(|e| IsolatorError::Pty(format!("take writer: {e}")))?;

    let (stdout_tx, stdout_rx) = mpsc::channel::<Vec<u8>>(CHANNEL_CAPACITY);
    let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(CHANNEL_CAPACITY);

    // Read pump
    tokio::task::spawn_blocking(move || {
        let mut reader = reader;
        let mut buf = vec![0u8; READ_BUF];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    // best-effort blocking_send; drop on closed receiver
                    if stdout_tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    });

    // Write pump
    tokio::task::spawn_blocking(move || {
        let mut writer = writer;
        // We hop between the async receiver and the blocking writer
        // via `blocking_recv`.
        while let Some(chunk) = stdin_rx.blocking_recv() {
            if writer.write_all(&chunk).is_err() {
                break;
            }
            let _ = writer.flush();
        }
    });

    Ok(PtyBridge {
        stdout_rx: Some(stdout_rx),
        stdin_tx: Some(stdin_tx),
        master,
        child,
    })
}

pub(crate) fn resize(master: &Arc<Mutex<Box<dyn MasterPty + Send>>>, cols: u16, rows: u16) -> Result<(), IsolatorError> {
    master
        .lock()
        .resize(PtySize {
            cols,
            rows,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| IsolatorError::Pty(format!("resize: {e}")))
}

pub(crate) fn kill(child: &Arc<Mutex<Box<dyn Child + Send + Sync>>>) -> Result<(), IsolatorError> {
    child
        .lock()
        .kill()
        .map_err(|e| IsolatorError::Pty(format!("kill: {e}")))
}

pub(crate) async fn wait(child: Arc<Mutex<Box<dyn Child + Send + Sync>>>) -> Result<ExitStatus, IsolatorError> {
    tokio::task::spawn_blocking(move || {
        let status = child.lock().wait().map_err(|e| IsolatorError::Pty(format!("wait: {e}")))?;
        let code = status.exit_code() as i32;
        Ok::<ExitStatus, IsolatorError>(ExitStatus::from_code(code))
    })
    .await
    .map_err(|e| IsolatorError::Pty(format!("join: {e}")))?
}
