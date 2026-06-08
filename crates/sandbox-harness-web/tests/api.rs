//! HTTP-level tests for the sandbox web API, driven in-process via
//! `tower::ServiceExt::oneshot` over the mock-backed harness.

use std::sync::Arc;

use atomr_agents_sandbox_harness::SandboxHarness;
use atomr_agents_sandbox_harness_web::{router, AppState};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

fn app() -> axum::Router {
    router(AppState { harness: Arc::new(SandboxHarness::local_default()) })
}

async fn json_body(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn post(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
async fn healthz_reports_backend() {
    let resp = app()
        .oneshot(Request::builder().uri("/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = json_body(resp).await;
    assert_eq!(v["backend"], "mock");
}

#[tokio::test]
async fn run_returns_flat_result() {
    let resp = app()
        .oneshot(post("/run", json!({ "language": "python", "code": "print(1)" })))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = json_body(resp).await;
    assert_eq!(v["success"], true);
    assert!(v["stdout"].as_str().unwrap().contains("[mock:Python]"));
}

#[tokio::test]
async fn create_exec_destroy_roundtrip() {
    let app = app();

    let resp = app
        .clone()
        .oneshot(post("/sandboxes", json!({ "profile": "full_stack" })))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let info = json_body(resp).await;
    let id = info["id"].as_str().unwrap().to_string();

    let resp = app
        .clone()
        .oneshot(post(
            &format!("/sandboxes/{id}/exec"),
            json!({ "language": "rust", "code": "fn main(){}" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(json_body(resp).await["success"], true);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/sandboxes/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn exec_on_unknown_id_is_404() {
    let resp = app()
        .oneshot(post(
            "/sandboxes/sbx-does-not-exist/exec",
            json!({ "language": "bash", "code": "echo hi" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn run_with_profile_language_mismatch_is_400() {
    let resp = app()
        .oneshot(post(
            "/run",
            json!({ "language": "rust", "code": "fn main(){}", "profile": "python_only" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
