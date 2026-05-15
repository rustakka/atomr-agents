//! REST route tests driven through `tower::ServiceExt::oneshot` — no
//! socket bind. Spec row: "web — REST CRUD + speaker PUT".

use std::sync::Arc;

use atomr_agents_stt_core::{Segment, SpeakerTag};
use atomr_agents_stt_harness::{ConversationStore, InMemoryConversationStore, SttConversation};
use atomr_agents_stt_harness_web::{WebConfig, WebServer};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

fn diarized_conversation(id: &str) -> SttConversation {
    let mut conv = SttConversation::new(id);
    conv.commit_segment(Segment {
        text: "hello".into(),
        start_ms: 0,
        end_ms: 0,
        words: vec![],
        speaker: Some(SpeakerTag { id: 0, label: None }),
        confidence: None,
    });
    conv
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    if bytes.is_empty() {
        return serde_json::Value::Null;
    }
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn list_get_delete_roundtrip() {
    let store = Arc::new(InMemoryConversationStore::new());
    store.put(&diarized_conversation("call-1")).await.unwrap();
    let server = WebServer::new(WebConfig::default(), store);

    // GET /api/conversations
    let resp = server
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/conversations")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let rows = body_json(resp).await;
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["id"], "call-1");
    assert_eq!(rows[0]["turn_count"], 1);
    assert_eq!(rows[0]["speaker_count"], 1);

    // GET /api/conversations/call-1
    let resp = server
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/conversations/call-1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let conv = body_json(resp).await;
    assert_eq!(conv["turns"].as_array().unwrap().len(), 1);

    // DELETE /api/conversations/call-1
    let resp = server
        .router()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/conversations/call-1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // GET again → 404
    let resp = server
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/conversations/call-1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn rename_speaker_persists_through_the_api() {
    let store = Arc::new(InMemoryConversationStore::new());
    store.put(&diarized_conversation("call-2")).await.unwrap();
    let server = WebServer::new(WebConfig::default(), store.clone());

    // PUT /api/conversations/call-2/speakers/0  { "label": "Alice" }
    let resp = server
        .router()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/conversations/call-2/speakers/0")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"label":"Alice"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let conv = body_json(resp).await;
    assert_eq!(conv["speaker_labels"]["0"], "Alice");

    // The rename is persisted in the store.
    let reloaded = store.get("call-2").await.unwrap().unwrap();
    assert_eq!(reloaded.effective_label(0), "Alice");
}

#[tokio::test]
async fn rename_missing_conversation_is_404() {
    let store = Arc::new(InMemoryConversationStore::new());
    let server = WebServer::new(WebConfig::default(), store);

    let resp = server
        .router()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/conversations/nope/speakers/0")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"label":"X"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn healthz_ok() {
    let store = Arc::new(InMemoryConversationStore::new());
    let server = WebServer::new(WebConfig::default(), store);
    let resp = server
        .router()
        .oneshot(Request::builder().uri("/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
