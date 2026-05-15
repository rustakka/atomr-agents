//! Static SPA asset serving.

use axum::body::Body;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

#[cfg(feature = "embed-ui")]
#[allow(unused_imports)]
use rust_embed::{Embed, RustEmbed};

#[cfg(feature = "embed-ui")]
#[derive(RustEmbed)]
#[folder = "ui/"]
struct SpaAssets;

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

#[cfg(not(feature = "embed-ui"))]
pub async fn serve_embedded(_uri: Uri) -> Response {
    let body = serde_json::json!({
        "ui": "not embedded",
        "hint": "build with --features embed-ui",
    });
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}
