//! `/ws` — live meetings-harness event stream.

use std::time::Duration;

use atomr_agents_meetings_harness::MeetingsHarnessEvent;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Query, State, WebSocketUpgrade};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;

use crate::AppState;

const HEARTBEAT: Duration = Duration::from_secs(15);

#[derive(Debug, Default, Deserialize)]
pub struct WsQuery {
    /// Reserved for future per-meeting filtering.
    pub meeting: Option<String>,
}

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

/// Forward a [`MeetingsEventStream`] into the server's broadcast
/// channel. Spawn this when a fresh harness run starts so its events
/// surface live on `/ws`.
///
/// [`MeetingsEventStream`]: atomr_agents_meetings_harness::MeetingsEventStream
pub async fn forward_events(
    mut stream: atomr_agents_meetings_harness::MeetingsEventStream,
    sink: tokio::sync::broadcast::Sender<MeetingsHarnessEvent>,
) {
    while let Some(event) = stream.recv().await {
        let _ = sink.send(event);
    }
}
