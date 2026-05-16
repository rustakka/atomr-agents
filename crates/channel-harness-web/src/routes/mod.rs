//! REST + WebSocket route registration.

pub mod channels;
pub mod threads;
pub mod webhook;

use axum::routing::{get, post};
use axum::Router;
use tower_http::cors::CorsLayer;

use crate::AppState;

pub fn build_router(state: AppState) -> Router {
    let api = Router::new()
        .route("/channels", get(channels::list).post(channels::create))
        .route(
            "/channels/:id",
            get(channels::get_one).delete(channels::delete_one),
        )
        .route(
            "/channels/:id/threads",
            get(threads::list).post(threads::create),
        )
        .route(
            "/threads/:id",
            get(threads::get_one).delete(threads::delete_one),
        )
        .route(
            "/threads/:id/messages",
            get(threads::list_messages).post(threads::send_message),
        )
        .with_state(state.clone());

    let webhook = Router::new()
        .route("/:provider/:channel_id", post(webhook::receive))
        .with_state(state.clone());

    let app = Router::new()
        .nest("/api", api)
        .nest("/webhook", webhook)
        .route("/ws", get(ws_handler_with_state))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(state);

    app.layer(CorsLayer::permissive())
}

async fn ws_handler_with_state(
    ws: axum::extract::WebSocketUpgrade,
    state: axum::extract::State<AppState>,
) -> axum::response::Response {
    crate::ws::ws_handler(ws, state).await
}

