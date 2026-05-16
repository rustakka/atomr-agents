//! REST + webhook smoke tests using `tower::ServiceExt::oneshot`.

use std::sync::Arc;

use atomr_agents_callable::FnCallable;
use atomr_agents_channel_core::memory::InMemoryProvider;
use atomr_agents_channel_core::{
    Callable, ChannelId, ChannelSpec, PeerId, ProviderKind, ThreadId, ThreadTarget,
};
use atomr_agents_core::Value;

fn encode(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            other => format!("%{:02X}", other as u32),
        })
        .collect()
}
use atomr_agents_channel_harness::ChannelHarness;
use atomr_agents_channel_harness_web::{WebConfig, WebServer};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn echo() -> Arc<dyn Callable> {
    Arc::new(FnCallable::labeled("echo", |input: Value, _ctx| async move {
        let text = input
            .get("user")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        Ok(serde_json::json!({ "text": format!("echo: {text}") }))
    }))
}

#[tokio::test]
async fn healthz_returns_ok() {
    let harness = Arc::new(ChannelHarness::in_memory());
    let server = WebServer::new(WebConfig::default(), harness);
    let router = server.router();
    let response = router
        .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn list_channels_and_threads_through_api() {
    let harness = Arc::new(ChannelHarness::in_memory());
    let channel_id = ChannelId::from("memory:web");
    let provider = Arc::new(InMemoryProvider::new(channel_id.clone()));
    harness
        .attach_provider(
            ChannelSpec::new(channel_id.clone(), ProviderKind::Memory),
            provider,
        )
        .await
        .unwrap();
    harness
        .open_thread(&channel_id, PeerId::from("alice"), ThreadTarget::callable(echo()))
        .await
        .unwrap();
    let thread_id = ThreadId::for_peer(&channel_id, &PeerId::from("alice"));

    let server = WebServer::new(WebConfig::default(), harness.clone());
    let router = server.router();

    let resp = router
        .clone()
        .oneshot(Request::get("/api/channels").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(json.as_array().unwrap().iter().any(|c| c["id"] == "memory:web"));

    let url = format!("/api/channels/{}/threads", encode(channel_id.as_str()));
    let resp = router
        .clone()
        .oneshot(Request::get(&url).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json.as_array().unwrap().len(), 1);
    assert_eq!(json[0]["id"], thread_id.as_str());
}

#[tokio::test]
async fn admin_send_via_post_messages() {
    let harness = Arc::new(ChannelHarness::in_memory());
    let channel_id = ChannelId::from("memory:web2");
    let provider = Arc::new(InMemoryProvider::new(channel_id.clone()));
    let mut sent = provider.sent_log();
    harness
        .attach_provider(
            ChannelSpec::new(channel_id.clone(), ProviderKind::Memory),
            provider,
        )
        .await
        .unwrap();
    harness
        .open_thread(&channel_id, PeerId::from("bob"), ThreadTarget::callable(echo()))
        .await
        .unwrap();
    let thread_id = ThreadId::for_peer(&channel_id, &PeerId::from("bob"));

    let server = WebServer::new(WebConfig::default(), harness.clone());
    let router = server.router();
    let url = format!("/api/threads/{}/messages", encode(thread_id.as_str()));
    let body = serde_json::json!({ "text": "hello from rest" }).to_string();
    let resp = router
        .oneshot(
            Request::post(&url)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let outbound = tokio::time::timeout(std::time::Duration::from_secs(2), sent.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outbound.content.as_text(), "hello from rest");

    // Verify list_messages now shows the outbound record.
    let url = format!("/api/threads/{}/messages?limit=10", encode(thread_id.as_str()));
    let resp = router_clone(&harness)
        .oneshot(Request::get(&url).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let arr = json.as_array().unwrap();
    assert!(!arr.is_empty());
}

fn router_clone(harness: &Arc<ChannelHarness>) -> axum::Router {
    WebServer::new(WebConfig::default(), harness.clone()).router()
}

#[tokio::test]
async fn unknown_channel_get_returns_404() {
    let harness = Arc::new(ChannelHarness::in_memory());
    let server = WebServer::new(WebConfig::default(), harness);
    let router = server.router();
    let resp = router
        .oneshot(
            Request::get("/api/channels/does-not-exist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn webhook_endpoint_rejects_unknown_channel() {
    let harness = Arc::new(ChannelHarness::in_memory());
    let server = WebServer::new(WebConfig::default(), harness);
    let router = server.router();
    let resp = router
        .oneshot(
            Request::post("/webhook/whatsapp/nope")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // ingest_webhook returns Err -> we map to 400
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
