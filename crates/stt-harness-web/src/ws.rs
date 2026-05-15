//! `/ws` — live STT-harness event stream.
//!
//! Clients connect and receive every [`SttHarnessEvent`] the server's
//! broadcast channel carries, serialized as JSON. Periodic ping frames
//! keep idle connections alive. Mirrors `atomr-dashboard`'s `ws.rs`.

use std::time::Duration;

use atomr_agents_stt_harness::SttHarnessEvent;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Query, State, WebSocketUpgrade};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;

use crate::AppState;

const HEARTBEAT: Duration = Duration::from_secs(15);

/// Optional query parameters on the `/ws` upgrade.
#[derive(Debug, Default, Deserialize)]
pub struct WsQuery {
    /// Reserved for future per-conversation filtering. `SttHarnessEvent`
    /// does not yet carry a conversation id, so today every subscriber
    /// receives every event.
    pub conversation: Option<String>,
}

/// Upgrade an HTTP request to a WebSocket and start relaying events.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<WsQuery>,
    State(state): State<AppState>,
) -> Response {
    ws.on_upgrade(move |socket| run_socket(socket, state, query))
}

async fn run_socket(socket: WebSocket, state: AppState, _query: WsQuery) {
    let mut rx = state.events.subscribe();
    let (mut sink, mut stream) = socket.split();
    let mut heartbeat = tokio::time::interval(HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            event = rx.recv() => {
                match event {
                    Ok(ev) => {
                        let Ok(body) = serde_json::to_string(&ev) else { continue };
                        if sink.send(Message::Text(body)).await.is_err() {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(skipped)) => {
                        let msg = serde_json::json!({ "kind": "lagged", "skipped": skipped });
                        let _ = sink.send(Message::Text(msg.to_string())).await;
                    }
                    Err(RecvError::Closed) => break,
                }
            }
            _ = heartbeat.tick() => {
                if sink.send(Message::Ping(Vec::new())).await.is_err() {
                    break;
                }
            }
            msg = stream.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
        }
    }
}

/// Forward an [`SttHarnessEvent`] subscriber stream into the server's
/// broadcast channel, so a running [`SttHarness`] surfaces live on
/// `/ws`. Spawn this alongside `WebServer::start`.
///
/// [`SttHarness`]: atomr_agents_stt_harness::SttHarness
pub async fn forward_events(
    mut stream: atomr_agents_stt_harness::SttEventStream,
    sink: tokio::sync::broadcast::Sender<SttHarnessEvent>,
) {
    while let Some(event) = stream.recv().await {
        // A send with no subscribers is fine — drop it.
        let _ = sink.send(event);
    }
}
