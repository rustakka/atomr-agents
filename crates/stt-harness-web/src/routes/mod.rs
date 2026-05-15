//! REST + WebSocket route registration. All handlers share
//! [`crate::AppState`].

pub mod conversations;

use axum::routing::{get, put};
use axum::Router;
use tower_http::cors::CorsLayer;

use crate::AppState;

/// Assemble the full router: the JSON API under `/api`, the `/ws`
/// stream, a health check, and — with `embed-ui` — the SPA fallback.
pub fn build_router(state: AppState) -> Router {
    let api = Router::new()
        .route("/conversations", get(conversations::list))
        .route(
            "/conversations/:id",
            get(conversations::get).delete(conversations::delete_one),
        )
        .route(
            "/conversations/:id/transcript.json",
            get(conversations::transcript_json),
        )
        .route(
            "/conversations/:id/speakers/:speaker_id",
            put(conversations::rename_speaker),
        )
        .with_state(state.clone());

    // The SPA fallback handler resolves per `embed-ui`: embedded
    // assets, or a JSON pointer to the Vite dev server.
    let app = Router::new()
        .nest("/api", api)
        .route("/ws", get(ws_handler_with_state))
        .route("/healthz", get(|| async { "ok" }))
        .fallback(crate::spa::serve_embedded)
        .with_state(state);

    app.layer(CorsLayer::permissive())
}

/// Thin adapter so `/ws` shares the single [`AppState`] router state.
async fn ws_handler_with_state(
    ws: axum::extract::WebSocketUpgrade,
    query: axum::extract::Query<crate::ws::WsQuery>,
    state: axum::extract::State<AppState>,
) -> axum::response::Response {
    crate::ws::ws_handler(ws, query, state).await
}
