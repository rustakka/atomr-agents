//! Integration tests for the deep-research web routes.
//!
//! Drives the router directly via `tower::ServiceExt::oneshot` so no
//! socket is bound.

use std::sync::Arc;

use atomr_agents_deep_research_harness::InMemoryResearchStore;
use atomr_agents_deep_research_harness_web::{AppState, WebConfig, WebServer};
use atomr_agents_web_search_core::{MockWebSearch, WebSearchHit};
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tokio::sync::broadcast;
use tower::ServiceExt;
use url::Url;

fn make_server() -> WebServer {
    let store = Arc::new(InMemoryResearchStore::new());
    let mock = Arc::new(MockWebSearch::new().with_fixture(
        "rust",
        vec![WebSearchHit::new(
            Url::parse("https://rust-lang.org/").unwrap(),
            "Rust homepage",
            "rust programming language",
        )],
    ));
    WebServer::new(WebConfig::default(), store, mock)
}

#[tokio::test]
async fn healthz_returns_ok() {
    let server = make_server();
    let app = server.router();
    let resp = app
        .oneshot(Request::builder().uri("/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 1024).await.unwrap();
    assert_eq!(&bytes[..], b"ok");
}

#[tokio::test]
async fn strategies_returns_all_ids() {
    let app = make_server().router();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/strategies")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 4 * 1024).await.unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 6);
    let names: Vec<&str> = arr.iter().map(|s| s.as_str().unwrap()).collect();
    assert!(names.contains(&"clarify-plan-search-verify"));
    assert!(names.contains(&"multi-agent-parallel"));
    assert!(names.contains(&"iterative-deepening"));
    assert!(names.contains(&"plan-and-execute"));
    assert!(names.contains(&"linear-write-critique"));
    assert!(names.contains(&"outline-first-section-fanout"));
}

#[tokio::test]
async fn start_run_persists_and_lists() {
    let server = make_server();
    let app = server.router();
    let body = serde_json::json!({
        "request": { "query": "compare rust frameworks", "depth": 1, "breadth": 2 },
        "strategy": "clarify-plan-search-verify",
        "max_iterations": 32,
    });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/research")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 4 * 1024).await.unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    let id = v.get("id").and_then(|x| x.as_str()).unwrap().to_string();
    assert!(!id.is_empty());

    // The run should now appear in /api/research.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/research")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let rows: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(rows.as_array().unwrap().iter().any(|r| r["id"] == id));

    // Fetch the full result.
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/research/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn unknown_strategy_rejected() {
    let app = make_server().router();
    let body = serde_json::json!({
        "request": { "query": "x" },
        "strategy": "no-such-strategy",
    });
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/research")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn missing_id_returns_404() {
    let app = make_server().router();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/research/no-such-id")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn shutdown_works() {
    // End-to-end: bind a real socket, then shut it down.
    let server = make_server();
    let handle = server.start().await.unwrap();
    let addr = handle.bound_addr;
    assert!(addr.port() > 0);
    handle.shutdown().await;
}

#[tokio::test]
async fn event_sender_is_clonable() {
    let server = make_server();
    let tx: broadcast::Sender<atomr_agents_deep_research_harness::DeepResearchEvent> = server.event_sender();
    let _ = AppState {
        store: Arc::new(InMemoryResearchStore::new()),
        search: Arc::new(MockWebSearch::new()),
        events: tx.clone(),
        supervisor: Arc::new(parking_lot::Mutex::new(Default::default())),
    };
}
