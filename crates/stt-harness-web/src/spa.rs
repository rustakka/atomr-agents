//! Static SPA asset serving.
//!
//! With the `embed-ui` feature, `ui/dist/` is baked into the binary
//! via `rust-embed` and served as an SPA (unknown paths fall back to
//! `index.html` so client-side routing works). Without the feature,
//! `/` returns a JSON pointer to the Vite dev server. Mirrors
//! `atomr-dashboard`'s `spa.rs`.

#[cfg(feature = "embed-ui")]
use axum::body::Body;
#[cfg(feature = "embed-ui")]
use axum::http::{header, StatusCode, Uri};
#[cfg(feature = "embed-ui")]
use axum::response::{IntoResponse, Response};

#[cfg(feature = "embed-ui")]
#[allow(unused_imports)]
use rust_embed::{Embed, RustEmbed};

#[cfg(feature = "embed-ui")]
#[derive(RustEmbed)]
#[folder = "ui/dist"]
struct SpaAssets;

/// Serve an embedded SPA asset, falling back to `index.html` for
/// client-routed paths.
#[cfg(feature = "embed-ui")]
pub async fn serve_embedded(uri: Uri) -> Response {
    let mut path = uri.path().trim_start_matches('/').to_string();
    if path.is_empty() || SpaAssets::get(&path).is_none() {
        path = "index.html".into();
    }
    match SpaAssets::get(&path) {
        Some(content) => {
            let mime = content.metadata.mimetype();
            Response::builder()
                .header(header::CONTENT_TYPE, mime)
                .body(Body::from(content.data.into_owned()))
                .unwrap()
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

/// Placeholder when the SPA is not embedded — point callers at the
/// Vite dev server.
#[cfg(not(feature = "embed-ui"))]
pub async fn serve_embedded() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "ui": "not embedded",
        "hint": "build with --features embed-ui, or run the Vite dev server: \
                 npm --prefix crates/stt-harness-web/ui run dev",
    }))
}
