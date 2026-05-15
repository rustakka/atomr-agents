//! REST route tests driven through `tower::ServiceExt::oneshot`.

use std::sync::Arc;

use atomr_agents_meetings_harness::{ActionStatus, InMemoryMeetingsStore, MeetingAnalysis, MeetingsStore};
use atomr_agents_meetings_harness_web::{WebConfig, WebServer};
use atomr_agents_stt_core::{Segment, SpeakerTag};
use atomr_agents_stt_harness::{ConversationStore, InMemoryConversationStore, SttConversation};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

fn seeded_analysis(id: &str) -> MeetingAnalysis {
    let mut a = MeetingAnalysis::new(id);
    a.attendees.push(atomr_agents_meetings_harness::Attendee {
        id: "att-1".into(),
        display_name: "Alice".into(),
        role: None,
        speaker_tags: vec![0],
        email: None,
    });
    a.actions.push(atomr_agents_meetings_harness::Action {
        id: "act-1".into(),
        description: "Ship".into(),
        owner_attendee_id: Some("att-1".into()),
        due_iso: None,
        supporting_quote: None,
        source_turn_index: None,
        status: ActionStatus::Open,
    });
    a
}

fn diarized_conversation(id: &str) -> SttConversation {
    let mut conv = SttConversation::new(id);
    conv.commit_segment(Segment {
        text: "hello".into(),
        start_ms: 0,
        end_ms: 1_000,
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

fn make_server() -> WebServer {
    let analyses: Arc<dyn MeetingsStore> = Arc::new(InMemoryMeetingsStore::new());
    let transcripts: Arc<dyn ConversationStore> = Arc::new(InMemoryConversationStore::new());
    WebServer::new(WebConfig::default(), analyses, transcripts)
}

#[tokio::test]
async fn list_get_delete_roundtrip() {
    let analyses = Arc::new(InMemoryMeetingsStore::new());
    let transcripts: Arc<dyn ConversationStore> = Arc::new(InMemoryConversationStore::new());
    analyses.put(&seeded_analysis("m-1")).await.unwrap();
    let server = WebServer::new(WebConfig::default(), analyses, transcripts);

    let resp = server
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/meetings")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let rows = body_json(resp).await;
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["id"], "m-1");
    assert_eq!(rows[0]["attendee_count"], 1);
    assert_eq!(rows[0]["action_count"], 1);

    let resp = server
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/meetings/m-1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["attendees"].as_array().unwrap().len(), 1);

    let resp = server
        .router()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/meetings/m-1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let resp = server
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/meetings/m-1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn rename_attendee_persists_through_the_api() {
    let analyses = Arc::new(InMemoryMeetingsStore::new());
    let transcripts: Arc<dyn ConversationStore> = Arc::new(InMemoryConversationStore::new());
    analyses.put(&seeded_analysis("m-2")).await.unwrap();
    let server = WebServer::new(WebConfig::default(), analyses.clone(), transcripts);

    let resp = server
        .router()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/meetings/m-2/attendees/att-1")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"display_name":"Alicia","role":"PM"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["attendees"][0]["display_name"], "Alicia");
    assert_eq!(body["attendees"][0]["role"], "PM");

    let reloaded = analyses.get("m-2").await.unwrap().unwrap();
    assert_eq!(reloaded.attendees[0].display_name, "Alicia");
}

#[tokio::test]
async fn update_action_status_persists_through_the_api() {
    let analyses = Arc::new(InMemoryMeetingsStore::new());
    let transcripts: Arc<dyn ConversationStore> = Arc::new(InMemoryConversationStore::new());
    analyses.put(&seeded_analysis("m-3")).await.unwrap();
    let server = WebServer::new(WebConfig::default(), analyses.clone(), transcripts);

    let resp = server
        .router()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/api/meetings/m-3/actions/act-1")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"status":"done"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["actions"][0]["status"], "done");

    let reloaded = analyses.get("m-3").await.unwrap().unwrap();
    assert_eq!(reloaded.actions[0].status, ActionStatus::Done);
}

#[tokio::test]
async fn missing_meeting_is_404() {
    let server = make_server();
    let resp = server
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/meetings/nope")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn healthz_ok() {
    let server = make_server();
    let resp = server
        .router()
        .oneshot(Request::builder().uri("/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn list_transcripts_returns_summary_rows() {
    let analyses = Arc::new(InMemoryMeetingsStore::new());
    let transcripts = Arc::new(InMemoryConversationStore::new());
    transcripts.put(&diarized_conversation("c-1")).await.unwrap();
    let transcripts_dyn: Arc<dyn ConversationStore> = transcripts.clone();
    let server = WebServer::new(WebConfig::default(), analyses, transcripts_dyn);

    let resp = server
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/transcripts")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let rows = body_json(resp).await;
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["id"], "c-1");
}

#[tokio::test]
async fn trigger_run_batch_persists_an_analysis() {
    let analyses: Arc<dyn MeetingsStore> = Arc::new(InMemoryMeetingsStore::new());
    let transcripts = Arc::new(InMemoryConversationStore::new());
    let mut conv = SttConversation::new("conv-x");
    conv.commit_segment(Segment {
        text: "We'll ship the proposal.".into(),
        start_ms: 0,
        end_ms: 2_000,
        words: vec![],
        speaker: Some(SpeakerTag { id: 0, label: None }),
        confidence: None,
    });
    transcripts.put(&conv).await.unwrap();
    let server = WebServer::new(
        WebConfig::default(),
        analyses.clone(),
        transcripts.clone() as Arc<dyn ConversationStore>,
    );

    let resp = server
        .router()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/meetings/conv-x/run")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"mode":"batch","model_id":"claude-opus-4-7"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // The run task is spawned in the background; poll briefly until the
    // analysis lands in the final state.
    for _ in 0..50 {
        if let Some(a) = analyses.get("conv-x").await.unwrap() {
            if a.state == atomr_agents_meetings_harness::AnalysisState::Final {
                assert!(!a.notes.is_empty());
                return;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("background run did not finalize within timeout");
}

#[tokio::test]
async fn stop_run_returns_accepted() {
    let server = make_server();
    let resp = server
        .router()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/meetings/anything/stop")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
}
