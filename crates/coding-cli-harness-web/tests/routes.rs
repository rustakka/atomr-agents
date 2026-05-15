//! Smoke tests for the REST routes — uses `tower::ServiceExt::oneshot`
//! to drive the router in-process without binding a socket.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;

use atomr_agents_coding_cli_core::{
    CliCommand, CliEventParser, CliRequest, CliVendor, CliVendorKind, CodingCliEvent,
    ConceptProjection, FinishReason, MapperError, ParseError,
};
use atomr_agents_coding_cli_harness::{
    CodingCliHarness, CodingCliHarnessSpec, InMemoryRunStore, VendorRegistry,
};
use atomr_agents_coding_cli_harness_web::{WebServer, WebConfig};
use atomr_agents_coding_cli_isolator::LocalIsolator;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tempfile::TempDir;
use tower::ServiceExt;

#[derive(Clone)]
struct MockVendor;

struct MockParser;
impl CliEventParser for MockParser {
    fn parse_line(&mut self, line: &str) -> Result<Vec<CodingCliEvent>, ParseError> {
        let s = line.trim();
        if s.is_empty() {
            return Ok(vec![]);
        }
        Ok(vec![CodingCliEvent::RunFinished {
            reason: FinishReason::Completed,
            result_text: Some(s.to_string()),
        }])
    }
    fn flush(&mut self) -> Result<Vec<CodingCliEvent>, ParseError> {
        Ok(vec![])
    }
}

#[async_trait]
impl CliVendor for MockVendor {
    fn kind(&self) -> CliVendorKind {
        CliVendorKind::Other("mock".into())
    }
    fn label(&self) -> &str {
        "Mock"
    }
    fn build_headless_command(&self, _req: &CliRequest, workdir: &Path) -> CliCommand {
        CliCommand::new("/usr/bin/printf", workdir).arg("ok\\n")
    }
    fn build_interactive_command(&self, _req: &CliRequest, workdir: &Path) -> CliCommand {
        CliCommand::new("/usr/bin/printf", workdir).arg("x\\n")
    }
    fn new_parser(&self) -> Box<dyn CliEventParser> {
        Box::new(MockParser)
    }
    async fn materialize_config(
        &self,
        _p: &ConceptProjection,
        _w: &Path,
    ) -> Result<(), MapperError> {
        Ok(())
    }
    async fn is_available(&self) -> bool {
        true
    }
}

fn build_server() -> (WebServer, TempDir) {
    let dir = TempDir::new().unwrap();
    let harness = CodingCliHarness::new(
        CodingCliHarnessSpec::default(),
        VendorRegistry::new().with(Arc::new(MockVendor)),
        Arc::new(LocalIsolator::new()),
        Arc::new(InMemoryRunStore::new()),
    );
    (WebServer::new(WebConfig::default(), Arc::new(harness)), dir)
}

#[tokio::test]
async fn healthz_returns_ok() {
    let (server, _dir) = build_server();
    let app = server.router();
    let resp = app
        .oneshot(Request::builder().uri("/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), 64).await.unwrap();
    assert_eq!(&body[..], b"ok");
}

#[tokio::test]
async fn vendors_lists_mock() {
    let (server, _dir) = build_server();
    let app = server.router();
    let resp = app
        .oneshot(Request::builder().uri("/api/cli/vendors").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let vendors = v["vendors"].as_array().unwrap();
    assert!(vendors.iter().any(|x| x.as_str() == Some("mock")));
}

#[tokio::test]
async fn start_run_returns_accepted() {
    let (server, dir) = build_server();
    let app = server.router();
    let req = CliRequest::new(CliVendorKind::Other("mock".into()), dir.path(), "go");
    let body = serde_json::to_vec(&req).unwrap();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/cli/runs")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(v["run_id"].as_str().unwrap().starts_with("cli-run-"));
}

#[tokio::test]
async fn list_runs_starts_empty() {
    let (server, _dir) = build_server();
    let app = server.router();
    let resp = app
        .oneshot(Request::builder().uri("/api/cli/runs").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(v["runs"].is_array());
}

#[tokio::test]
async fn unknown_run_returns_404() {
    let (server, _dir) = build_server();
    let app = server.router();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/cli/runs/cli-run-nope")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn sessions_list_empty_initially() {
    let (server, _dir) = build_server();
    let app = server.router();
    let resp = app
        .oneshot(Request::builder().uri("/api/cli/sessions").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(v["sessions"].as_array().unwrap().is_empty());
}
