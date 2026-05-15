//! REST + SSE route registration.

pub mod research;

use axum::routing::{delete, get, post};
use axum::Router;
use tower_http::cors::CorsLayer;

use crate::AppState;

/// Assemble the full router.
pub fn build_router(state: AppState) -> Router {
    let api = Router::new()
        .route("/research", get(research::list).post(research::start))
        .route(
            "/research/:id",
            get(research::get_one).delete(research::delete_one),
        )
        .route("/research/:id/stop", post(research::stop_run))
        .route("/research/events", get(crate::sse::sse_events))
        .route("/strategies", get(research::list_strategies))
        .with_state(state.clone());

    Router::new()
        .nest("/api", api)
        .route("/healthz", get(|| async { "ok" }))
        .fallback(crate::spa::serve_embedded)
        .with_state(state)
        .layer(CorsLayer::permissive())
}

// Suppress an axum::routing::delete unused import warning when the
// feature set is reduced; the symbol is still referenced via the macro.
#[allow(dead_code)]
fn _ensure_delete_in_scope() -> axum::routing::MethodRouter<()> {
    delete(|| async { "" })
}
