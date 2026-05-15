//! WebSocket terminal bridge for interactive tmux-wrapped sessions.

use atomr_agents_coding_cli_core::CliSessionId;
use atomr_agents_coding_cli_harness::SessionEvent;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::Response;
use serde::Deserialize;
use tracing::warn;

use crate::AppState;

/// `GET /api/cli/sessions/:id/io` — WebSocket upgrade. Binary frames
/// flow in both directions for PTY bytes; text frames carry a small
/// control protocol (resize).
pub async fn session_ws(
    ws: WebSocketUpgrade,
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, CliSessionId::from(id), state))
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ControlFrame {
    Resize { cols: u16, rows: u16 },
    Detach,
}

async fn handle_socket(mut socket: WebSocket, id: CliSessionId, state: AppState) {
    let Some(handle) = state.harness.sessions().get(&id) else {
        let _ = socket
            .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                code: 1011,
                reason: "session not found".into(),
            })))
            .await;
        return;
    };

    let mut rx = handle.subscribe();
    let input = handle.input.clone();

    loop {
        tokio::select! {
            // PTY → client
            msg = rx.recv() => {
                match msg {
                    Ok(SessionEvent::Bytes(bytes)) => {
                        if socket.send(Message::Binary(bytes)).await.is_err() {
                            break;
                        }
                    }
                    Ok(SessionEvent::Exited { code }) => {
                        let body = serde_json::json!({"kind":"exited","code":code}).to_string();
                        let _ = socket.send(Message::Text(body)).await;
                        let _ = socket.send(Message::Close(None)).await;
                        break;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            // client → PTY
            client = socket.recv() => {
                match client {
                    Some(Ok(Message::Binary(bytes))) => {
                        let _ = input.send(atomr_agents_coding_cli_harness::SessionTransport::Stdin(bytes)).await;
                    }
                    Some(Ok(Message::Text(s))) => {
                        match serde_json::from_str::<ControlFrame>(&s) {
                            Ok(ControlFrame::Resize { cols, rows }) => {
                                let _ = handle.resize(cols, rows).await;
                            }
                            Ok(ControlFrame::Detach) => {
                                let _ = handle.detach().await;
                                break;
                            }
                            Err(e) => warn!(error = %e, "bad control frame; ignoring"),
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => {
                        warn!(error = %e, "websocket error");
                        break;
                    }
                }
            }
        }
    }
}
