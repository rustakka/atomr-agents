//! Integration tests driving the assembled Axum router directly (no socket).
//!
//! Each test scaffolds a throwaway host root with one fixture agent, builds the
//! real router via `AppState`, and exercises endpoints with `oneshot`.

use atomr_agents_host::{HostConfig, HostRuntime};
use atomr_agents_host_web::{routes, AppState};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

fn write(p: &std::path::Path, body: &str) {
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, body).unwrap();
}

fn fixture_agent(root: &std::path::Path) {
    let agent = root.join("agents").join("alpha");
    write(&agent.join("agent.yaml"), "id: alpha\nmodel: gpt-4o\n");
    write(
        &agent.join("SOUL.md"),
        "---\nidentity: Alpha\n---\nA terse agent.\n",
    );
    write(&agent.join("RULES.md"), "- be helpful\n");
    write(&agent.join("MEMORY.md"), "- fact one\n");
    write(&agent.join("USER.md"), "user is Matt\n");
}

async fn test_state(root: &std::path::Path) -> AppState {
    fixture_agent(root);
    let cfg = HostConfig::load(root).unwrap();
    let rt = HostRuntime::start(cfg).await.unwrap();
    AppState::new(rt)
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn healthz_ok() {
    let tmp = tempfile::tempdir().unwrap();
    let app = routes::build_router(test_state(tmp.path()).await);
    let resp = app
        .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn lists_agents_and_concepts() {
    let tmp = tempfile::tempdir().unwrap();
    let app = routes::build_router(test_state(tmp.path()).await);

    let resp = app
        .clone()
        .oneshot(Request::get("/api/agents").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    let agents = v["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0]["id"], "alpha");
    assert_eq!(agents[0]["model"], "gpt-4o");
    assert_eq!(agents[0]["rules_count"], 1);

    let resp = app
        .oneshot(Request::get("/api/concepts").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let v = body_json(resp).await;
    assert!(v["concepts"].as_array().unwrap().len() >= 15);
}

#[tokio::test]
async fn agent_detail_and_404() {
    let tmp = tempfile::tempdir().unwrap();
    let app = routes::build_router(test_state(tmp.path()).await);

    let resp = app
        .clone()
        .oneshot(Request::get("/api/agents/alpha").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    assert_eq!(v["id"], "alpha");
    assert_eq!(v["docs"]["soul"]["body"], "A terse agent.\n");

    let resp = app
        .oneshot(Request::get("/api/agents/ghost").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn doc_roundtrip_and_unknown_doc() {
    let tmp = tempfile::tempdir().unwrap();
    let app = routes::build_router(test_state(tmp.path()).await);

    // GET soul
    let resp = app
        .clone()
        .oneshot(
            Request::get("/api/agents/alpha/docs/soul")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let v = body_json(resp).await;
    assert_eq!(v["frontmatter"]["identity"], "Alpha");

    // PUT soul
    let update = json!({ "frontmatter": { "identity": "Beta" }, "body": "Changed.\n" });
    let resp = app
        .clone()
        .oneshot(
            Request::put("/api/agents/alpha/docs/soul")
                .header("content-type", "application/json")
                .body(Body::from(update.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // GET soul reflects the write
    let resp = app
        .clone()
        .oneshot(
            Request::get("/api/agents/alpha/docs/soul")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let v = body_json(resp).await;
    assert_eq!(v["frontmatter"]["identity"], "Beta");
    assert_eq!(v["body"], "Changed.\n");

    // unknown doc => 400
    let resp = app
        .oneshot(
            Request::get("/api/agents/alpha/docs/bogus")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn skill_create_and_list() {
    let tmp = tempfile::tempdir().unwrap();
    let app = routes::build_router(test_state(tmp.path()).await);

    let body = json!({ "id": "summarize", "name": "Summarize", "priority": 7, "keywords": ["tldr"] });
    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/agents/alpha/skills")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let resp = app
        .oneshot(
            Request::get("/api/agents/alpha/skills")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let v = body_json(resp).await;
    let skills = v["skills"].as_array().unwrap();
    assert!(skills.iter().any(|s| s["id"] == "summarize"));
}

#[tokio::test]
async fn cron_create_validate_and_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let app = routes::build_router(test_state(tmp.path()).await);

    // bad expression => 400
    let bad = json!({ "id": "x", "expression": "not-a-cron" });
    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/crons")
                .header("content-type", "application/json")
                .body(Body::from(bad.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // good expression => 201
    let good = json!({ "id": "heartbeat", "expression": "every:5m", "call": { "input": "ping" } });
    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/crons")
                .header("content-type", "application/json")
                .body(Body::from(good.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // delete => 204
    let resp = app
        .clone()
        .oneshot(
            Request::delete("/api/crons/heartbeat")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // delete missing => 404
    let resp = app
        .oneshot(
            Request::delete("/api/crons/heartbeat")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn branches_default_and_delete_main_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let app = routes::build_router(test_state(tmp.path()).await);

    let resp = app
        .clone()
        .oneshot(
            Request::get("/api/agents/alpha/branches")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    assert_eq!(v["current"], "main");

    // Deleting `main` without force is refused as a state conflict.
    let resp = app
        .clone()
        .oneshot(
            Request::delete("/api/agents/alpha/branches/main")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // Forking from a branch with no checkpoint is a precondition conflict, not 500.
    let body = json!({ "new": "experiment" });
    let resp = app
        .oneshot(
            Request::post("/api/agents/alpha/branches")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn config_get_and_invalid_put() {
    let tmp = tempfile::tempdir().unwrap();
    let app = routes::build_router(test_state(tmp.path()).await);

    let resp = app
        .clone()
        .oneshot(Request::get("/api/config").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    assert!(v["yaml"].is_string());

    // Non-mapping YAML must fail to parse into HostConfig.
    let bad = json!({ "yaml": "[1, 2, 3]\n" });
    let resp = app
        .oneshot(
            Request::put("/api/config")
                .header("content-type", "application/json")
                .body(Body::from(bad.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(resp.status().is_client_error() || resp.status().is_server_error());
}

#[tokio::test]
async fn events_appended_by_mutations() {
    let tmp = tempfile::tempdir().unwrap();
    let app = routes::build_router(test_state(tmp.path()).await);

    // A mutation that emits an event.
    let update = json!({ "frontmatter": {}, "body": "x\n" });
    let _ = app
        .clone()
        .oneshot(
            Request::put("/api/agents/alpha/docs/memory")
                .header("content-type", "application/json")
                .body(Body::from(update.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let resp = app
        .oneshot(Request::get("/api/events").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let v = body_json(resp).await;
    let events = v["events"].as_array().unwrap();
    assert!(events.iter().any(|e| e["kind"] == "doc.saved"));
}
