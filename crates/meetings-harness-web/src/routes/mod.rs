//! REST + WebSocket route registration. All handlers share
//! [`crate::AppState`].

pub mod meetings;

use axum::routing::{get, patch, post, put};
use axum::Router;
use tower_http::cors::CorsLayer;

use crate::AppState;

/// Assemble the full router: the JSON API under `/api`, the `/ws`
/// stream, a health check, and — with `embed-ui` — the SPA fallback.
pub fn build_router(state: AppState) -> Router {
    let api = Router::new()
        .route("/meetings", get(meetings::list))
        .route(
            "/meetings/:id",
            get(meetings::get).delete(meetings::delete_one),
        )
        .route(
            "/meetings/:id/attendees/:attendee_id",
            put(meetings::rename_attendee),
        )
        .route("/meetings/:id/actions/:action_id", patch(meetings::update_action))
        .route("/meetings/:id/run", post(meetings::trigger_run))
        .route("/meetings/:id/stop", post(meetings::stop_run))
        .route("/transcripts", get(meetings::list_transcripts))
        .with_state(state.clone());

    let app = Router::new()
        .nest("/api", api)
        .route("/ws", get(ws_handler_with_state))
        .route("/healthz", get(|| async { "ok" }))
        .fallback(crate::spa::serve_embedded)
        .with_state(state);

    app.layer(CorsLayer::permissive())
}

async fn ws_handler_with_state(
    ws: axum::extract::WebSocketUpgrade,
    query: axum::extract::Query<crate::ws::WsQuery>,
    state: axum::extract::State<AppState>,
) -> axum::response::Response {
    crate::ws::ws_handler(ws, query, state).await
}
