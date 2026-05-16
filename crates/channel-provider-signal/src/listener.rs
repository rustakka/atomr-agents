//! Long-running task that owns the JSON-RPC socket and demuxes
//! responses + notifications.
//!
//! Architecture:
//!
//! - A single reader task reads line-framed JSON from the socket.
//!   - Frames with `id` are responses → looked up in a shared
//!     pending-map and forwarded via [`oneshot`] to the awaiting
//!     caller.
//!   - Frames without `id` (notifications, e.g. method `"receive"`)
//!     are parsed into [`InboundMessage`]s and sent to the harness
//!     inbound channel.
//! - A single writer task drains an internal mpsc of pre-serialized
//!   frames and writes them to the socket. Multiple `send()` callers
//!   contend on the mpsc, not the socket, so we don't need a tokio
//!   `Mutex` held across awaits.
//!
//! Both tasks observe the shared `stop: Arc<AtomicBool>` flag and exit
//! cleanly when set.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use atomr_agents_channel_core::{ChannelError, ChannelId, InboundMessage, Result};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

use crate::rpc::{parse_envelope, JsonRpcResponse};

/// Shared state between the request side and the reader task.
///
/// Keys are JSON-RPC request ids; values are oneshot senders that
/// resolve the awaiting `send()` call.
pub(crate) type PendingMap = Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>;

/// Handle the public provider keeps for outbound writes + response
/// correlation. Clonable.
#[derive(Clone)]
pub(crate) struct ListenerHandle {
    write_tx: mpsc::Sender<Vec<u8>>,
    pending: PendingMap,
}

impl ListenerHandle {
    /// Register a pending request id and obtain the receiver to await
    /// its response.
    pub(crate) fn register(&self, id: String) -> oneshot::Receiver<Value> {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        rx
    }

    /// Discard a pending request (e.g. if the write failed).
    pub(crate) fn forget(&self, id: &str) {
        self.pending.lock().unwrap().remove(id);
    }

    /// Enqueue a JSON frame for the writer task. A trailing newline
    /// is appended.
    pub(crate) async fn write(&self, frame: Vec<u8>) -> Result<()> {
        self.write_tx
            .send(frame)
            .await
            .map_err(|_| ChannelError::transport("signal writer dropped"))
    }
}

/// Spin up the reader+writer tasks for an already-connected socket.
///
/// Returns a [`ListenerHandle`] (used by `send`) and the underlying
/// `JoinHandle` (joined by `ProviderHandle`).
pub(crate) fn spawn<R, W>(
    reader: R,
    writer: W,
    default_channel_id: ChannelId,
    inbound_tx: mpsc::Sender<InboundMessage>,
    stop: Arc<AtomicBool>,
) -> (ListenerHandle, tokio::task::JoinHandle<()>)
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
    let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>(64);
    let handle = ListenerHandle {
        write_tx,
        pending: pending.clone(),
    };

    let reader_stop = stop.clone();
    let writer_stop = stop.clone();
    let reader_pending = pending.clone();

    let writer_task = tokio::spawn(writer_loop(writer, write_rx, writer_stop));
    let join = tokio::spawn(async move {
        reader_loop(
            reader,
            inbound_tx,
            reader_pending,
            default_channel_id,
            reader_stop,
        )
        .await;
        // Once the reader exits, drop the writer too.
        writer_task.abort();
        let _ = writer_task.await;
    });

    (handle, join)
}

async fn reader_loop<R>(
    reader: R,
    inbound_tx: mpsc::Sender<InboundMessage>,
    pending: PendingMap,
    default_channel_id: ChannelId,
    stop: Arc<AtomicBool>,
) where
    R: AsyncRead + Send + Unpin + 'static,
{
    let buf = BufReader::new(reader);
    let mut lines = buf.lines();

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        tokio::select! {
            biased;
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                if stop.load(Ordering::Relaxed) { break; }
            }
            next = lines.next_line() => {
                match next {
                    Ok(Some(line)) => {
                        if line.trim().is_empty() {
                            continue;
                        }
                        if let Err(e) = handle_frame(
                            &line,
                            &inbound_tx,
                            &pending,
                            &default_channel_id,
                        ).await {
                            tracing::warn!(target: "signal", error = %e, line = %line, "frame handling failed");
                        }
                        if inbound_tx.is_closed() {
                            break;
                        }
                    }
                    Ok(None) => {
                        tracing::info!(target: "signal", "signal-cli socket closed by peer");
                        break;
                    }
                    Err(e) => {
                        tracing::warn!(target: "signal", error = %e, "signal-cli read error");
                        break;
                    }
                }
            }
        }
    }

    // Notify any still-pending callers that we're shutting down.
    let mut map = pending.lock().unwrap();
    for (_, tx) in map.drain() {
        let _ = tx.send(serde_json::json!({
            "__atomr_listener_closed": true
        }));
    }
}

async fn handle_frame(
    line: &str,
    inbound_tx: &mpsc::Sender<InboundMessage>,
    pending: &PendingMap,
    default_channel_id: &ChannelId,
) -> Result<()> {
    let value: Value = serde_json::from_str(line).map_err(ChannelError::Serde)?;

    // Notification vs response: notifications have no `id`.
    let id_present = value.get("id").is_some_and(|v| !v.is_null());

    if id_present {
        // Response: route to the awaiting oneshot.
        let resp: JsonRpcResponse = serde_json::from_value(value.clone())?;
        let id_str = match resp.id.as_ref() {
            Some(Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => return Ok(()),
        };
        let tx = pending.lock().unwrap().remove(&id_str);
        if let Some(tx) = tx {
            let _ = tx.send(value);
        } else {
            tracing::debug!(target: "signal", id = %id_str, "received response for unknown id");
        }
        return Ok(());
    }

    // Notification.
    let method = value
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("");
    if method != "receive" {
        // Other JSON-RPC notifications (e.g. typing, receipt) — ignored.
        return Ok(());
    }
    let Some(envelope) = value.pointer("/params/envelope") else {
        return Ok(());
    };
    match parse_envelope(envelope, default_channel_id)? {
        Some(msg) => {
            if inbound_tx.send(msg).await.is_err() {
                return Err(ChannelError::transport("inbound channel closed"));
            }
        }
        None => {}
    }
    Ok(())
}

async fn writer_loop<W>(mut writer: W, mut rx: mpsc::Receiver<Vec<u8>>, stop: Arc<AtomicBool>)
where
    W: AsyncWrite + Send + Unpin + 'static,
{
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        tokio::select! {
            biased;
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                if stop.load(Ordering::Relaxed) { break; }
            }
            frame = rx.recv() => {
                match frame {
                    Some(mut bytes) => {
                        if !bytes.ends_with(b"\n") {
                            bytes.push(b'\n');
                        }
                        if let Err(e) = writer.write_all(&bytes).await {
                            tracing::warn!(target: "signal", error = %e, "write failed");
                            break;
                        }
                        if let Err(e) = writer.flush().await {
                            tracing::warn!(target: "signal", error = %e, "flush failed");
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    }
}
