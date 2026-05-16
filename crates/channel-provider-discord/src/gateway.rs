//! Discord Gateway WebSocket handler.
//!
//! The Gateway is Discord's real-time event bus. Each connection
//! goes through the lifecycle:
//!
//! 1. Server sends `HELLO (op 10)` with `heartbeat_interval` ms.
//! 2. Client sends `IDENTIFY (op 2)` with bot token + intents.
//! 3. Server sends `READY (op 0, t = "READY")` followed by event
//!    dispatches like `MESSAGE_CREATE`.
//! 4. Client must send `HEARTBEAT (op 1)` with the last seen sequence
//!    number `s` every `heartbeat_interval` ms.
//! 5. Server acknowledges with `HEARTBEAT_ACK (op 11)`.
//!
//! For this crate's purposes we only care about `MESSAGE_CREATE`
//! dispatches; everything else is ignored. We do not implement RESUME
//! (a disconnect just terminates the provider task — the harness can
//! restart it).
//!
//! Reference: <https://discord.com/developers/docs/topics/gateway>

#![cfg(feature = "gateway")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use atomr_agents_channel_core::{ChannelError, InboundMessage, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::config::DiscordConfig;
use crate::parse::parse_message_create;

const OP_DISPATCH: u64 = 0;
const OP_HEARTBEAT: u64 = 1;
const OP_IDENTIFY: u64 = 2;
const OP_RECONNECT: u64 = 7;
const OP_INVALID_SESSION: u64 = 9;
const OP_HELLO: u64 = 10;
const OP_HEARTBEAT_ACK: u64 = 11;

/// Open a Gateway connection and forward `MESSAGE_CREATE` events to
/// `inbound_tx` until `stop` is set or the connection drops.
pub(crate) async fn run(
    config: DiscordConfig,
    inbound_tx: mpsc::Sender<InboundMessage>,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    let url = config.gateway_url().to_string();
    tracing::debug!(target: "discord", "connecting to gateway {url}");

    let (ws, _resp) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|e| ChannelError::transport(format!("gateway connect: {e}")))?;

    let (mut sink, mut source) = ws.split();

    // Pull HELLO to discover heartbeat interval.
    let hello = match source.next().await {
        Some(Ok(Message::Text(t))) => t,
        Some(Ok(Message::Binary(b))) => String::from_utf8_lossy(&b).to_string(),
        Some(Ok(other)) => {
            return Err(ChannelError::transport(format!(
                "gateway: expected HELLO, got {other:?}"
            )));
        }
        Some(Err(e)) => {
            return Err(ChannelError::transport(format!("gateway recv HELLO: {e}")));
        }
        None => return Err(ChannelError::transport("gateway closed before HELLO")),
    };
    let hello: Value = serde_json::from_str(&hello)
        .map_err(|e| ChannelError::transport(format!("gateway HELLO not JSON: {e}")))?;
    if hello.get("op").and_then(|v| v.as_u64()) != Some(OP_HELLO) {
        return Err(ChannelError::transport(format!(
            "gateway: expected HELLO op {OP_HELLO}, got {hello}"
        )));
    }
    let heartbeat_ms = hello
        .get("d")
        .and_then(|d| d.get("heartbeat_interval"))
        .and_then(|v| v.as_u64())
        .ok_or_else(|| ChannelError::transport("gateway HELLO missing heartbeat_interval"))?;

    // Send IDENTIFY.
    let identify = json!({
        "op": OP_IDENTIFY,
        "d": {
            "token": config.bot_token,
            "intents": config.intents(),
            "properties": {
                "os": std::env::consts::OS,
                "browser": "atomr-agents",
                "device": "atomr-agents",
            }
        }
    });
    sink.send(Message::Text(identify.to_string()))
        .await
        .map_err(|e| ChannelError::transport(format!("gateway IDENTIFY: {e}")))?;

    // Heartbeat task. The sink is exclusively owned by the
    // `sink_task` forwarder; both the heartbeat task and the main
    // receive loop push frames through `sink_tx`.
    let seq = Arc::new(parking_lot::Mutex::new(None::<i64>));
    let seq_for_hb = seq.clone();
    let stop_for_hb = stop.clone();

    let (sink_tx, mut sink_rx) = tokio::sync::mpsc::channel::<Message>(8);
    let stop_for_sink = stop.clone();
    let sink_task = tokio::spawn(async move {
        loop {
            if stop_for_sink.load(Ordering::Relaxed) {
                let _ = sink.close().await;
                return;
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(250)) => {}
                next = sink_rx.recv() => {
                    match next {
                        Some(Message::Close(_)) => {
                            let _ = sink.close().await;
                            return;
                        }
                        Some(m) => {
                            if sink.send(m).await.is_err() {
                                return;
                            }
                        }
                        None => {
                            let _ = sink.close().await;
                            return;
                        }
                    }
                }
            }
        }
    });

    let sink_for_hb = sink_tx.clone();
    let heartbeat_task = tokio::spawn(async move {
        // First heartbeat is delayed by a fraction of the interval per
        // the Discord docs ("jitter"), but a fixed delay is fine for v1.
        let interval = Duration::from_millis(heartbeat_ms);
        loop {
            tokio::time::sleep(interval).await;
            if stop_for_hb.load(Ordering::Relaxed) {
                return;
            }
            let s = *seq_for_hb.lock();
            let frame = json!({"op": OP_HEARTBEAT, "d": s});
            if sink_for_hb
                .send(Message::Text(frame.to_string()))
                .await
                .is_err()
            {
                return;
            }
        }
    });

    // Consume the source. Block out on a select with the stop flag so
    // shutdown is responsive even when the gateway is quiet.
    let result: Result<()> = loop {
        if stop.load(Ordering::Relaxed) {
            break Ok(());
        }
        let next = tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(250)) => continue,
            n = source.next() => n,
        };
        let Some(msg) = next else {
            break Err(ChannelError::transport("gateway closed"));
        };
        let msg = match msg {
            Ok(m) => m,
            Err(e) => break Err(ChannelError::transport(format!("gateway recv: {e}"))),
        };
        let text = match msg {
            Message::Text(t) => t,
            Message::Binary(b) => String::from_utf8_lossy(&b).to_string(),
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
            Message::Close(_) => break Ok(()),
        };
        let v: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_e) => continue,
        };
        let op = v.get("op").and_then(|x| x.as_u64()).unwrap_or(u64::MAX);
        if let Some(s) = v.get("s").and_then(|x| x.as_i64()) {
            *seq.lock() = Some(s);
        }
        match op {
            OP_HEARTBEAT_ACK => {}
            OP_HEARTBEAT => {
                // Server-requested immediate heartbeat.
                let s = *seq.lock();
                let frame = json!({"op": OP_HEARTBEAT, "d": s});
                let _ = sink_tx.send(Message::Text(frame.to_string())).await;
            }
            OP_RECONNECT | OP_INVALID_SESSION => {
                break Err(ChannelError::transport(format!(
                    "gateway requested disconnect (op {op})"
                )));
            }
            OP_DISPATCH => {
                let t = v.get("t").and_then(|x| x.as_str()).unwrap_or("");
                if t == "MESSAGE_CREATE" {
                    let d = v.get("d").cloned().unwrap_or(Value::Null);
                    let expect = config.discord_channel_id.as_deref();
                    match parse_message_create(
                        &d,
                        &config.default_channel_id,
                        expect,
                    ) {
                        Ok(Some(inbound)) => {
                            if inbound_tx.send(inbound).await.is_err() {
                                break Ok(());
                            }
                        }
                        Ok(None) => {}
                        Err(e) => {
                            tracing::debug!(target: "discord", "drop bad MESSAGE_CREATE: {e}");
                        }
                    }
                }
                // All other dispatch events ignored.
            }
            _ => {}
        }
    };

    // Shutdown: tell the sink task to close.
    let _ = sink_tx.send(Message::Close(None)).await;
    drop(sink_tx);
    let _ = sink_task.await;
    heartbeat_task.abort();
    let _ = heartbeat_task.await;

    result
}
