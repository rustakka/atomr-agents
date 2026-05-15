//! End-to-end integration of `CodingCliHarness` headless mode against
//! a mock vendor that emits a known NDJSON stream via `printf`.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;

use atomr_agents_callable::Callable;
use atomr_agents_coding_cli_core::{
    CliCommand, CliEventParser, CliRequest, CliVendor, CliVendorKind, CodingCliEvent,
    ConceptProjection, FinishReason, MapperError, ParseError, RunMode,
};
use atomr_agents_coding_cli_harness::{
    CliRunStore as _, CodingCliHarness, CodingCliHarnessSpec, InMemoryRunStore, VendorRegistry,
};
use atomr_agents_coding_cli_isolator::LocalIsolator;
use atomr_agents_core::{
    CallCtx, IterationBudget, MoneyBudget, TimeBudget, TokenBudget,
};
use tempfile::TempDir;

/// Self-contained mock vendor that emits a canned NDJSON stream via
/// `printf`, parses it via a tiny Claude-compatible parser.
#[derive(Clone)]
struct MockVendor {
    label: String,
    ndjson: String,
}

impl MockVendor {
    fn new(ndjson: &str) -> Self {
        Self {
            label: "Mock".into(),
            ndjson: ndjson.to_string(),
        }
    }
}

struct MockParser;
impl CliEventParser for MockParser {
    fn parse_line(&mut self, line: &str) -> Result<Vec<CodingCliEvent>, ParseError> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(vec![]);
        }
        let v: serde_json::Value = serde_json::from_str(trimmed)?;
        match v.get("type").and_then(|x| x.as_str()).unwrap_or("") {
            "delta" => Ok(vec![CodingCliEvent::AssistantTextDelta {
                text: v.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            }]),
            "result" => Ok(vec![CodingCliEvent::RunFinished {
                reason: FinishReason::Completed,
                result_text: v.get("text").and_then(|x| x.as_str()).map(|s| s.to_string()),
            }]),
            _ => Ok(vec![CodingCliEvent::RawVendorEvent {
                vendor: CliVendorKind::Other("mock".into()),
                payload: v,
            }]),
        }
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
        &self.label
    }
    fn build_headless_command(&self, _req: &CliRequest, workdir: &Path) -> CliCommand {
        // `printf` honors `\n` so newline-separated NDJSON arrives as
        // line-buffered chunks downstream.
        CliCommand::new("/usr/bin/printf", workdir).arg(&self.ndjson)
    }
    fn build_interactive_command(&self, _req: &CliRequest, workdir: &Path) -> CliCommand {
        CliCommand::new("/usr/bin/printf", workdir).arg("interactive\n")
    }
    fn new_parser(&self) -> Box<dyn CliEventParser> {
        Box::new(MockParser)
    }
    async fn materialize_config(
        &self,
        _p: &ConceptProjection,
        _workdir: &Path,
    ) -> Result<(), MapperError> {
        Ok(())
    }
    async fn is_available(&self) -> bool {
        true
    }
}

fn make_harness(vendor: MockVendor) -> CodingCliHarness {
    CodingCliHarness::new(
        CodingCliHarnessSpec::default(),
        VendorRegistry::new().with(Arc::new(vendor)),
        Arc::new(LocalIsolator::new()),
        Arc::new(InMemoryRunStore::new()),
    )
}

fn ctx() -> CallCtx {
    CallCtx {
        agent_id: None,
        tokens: TokenBudget::new(10_000),
        time: TimeBudget::new(std::time::Duration::from_secs(5)),
        money: MoneyBudget::from_usd(1.0),
        iterations: IterationBudget::new(4),
        trace: vec![],
    }
}

#[tokio::test]
async fn headless_run_streams_events_and_completes() {
    let dir = TempDir::new().unwrap();
    let ndjson = "{\"type\":\"delta\",\"text\":\"Hello \"}\n{\"type\":\"delta\",\"text\":\"world\"}\n{\"type\":\"result\",\"text\":\"Hello world\"}\n";
    let harness = make_harness(MockVendor::new(ndjson));

    let req = CliRequest::new(
        CliVendorKind::Other("mock".into()),
        dir.path(),
        "ignored",
    );

    let mut stream = harness.events();
    let task = tokio::spawn(async move {
        let mut texts = Vec::new();
        let mut got_finished = false;
        while let Some(ev) = stream.recv().await {
            match ev {
                CodingCliEvent::AssistantTextDelta { text } => texts.push(text),
                CodingCliEvent::RunFinished { .. } => {
                    got_finished = true;
                    break;
                }
                _ => {}
            }
        }
        (texts, got_finished)
    });

    let result = harness.run(req).await.expect("run ok");
    let (texts, got_finished) = task.await.unwrap();

    assert_eq!(texts.concat(), "Hello world");
    assert!(got_finished);
    assert_eq!(result.final_text, "Hello world");
    assert!(matches!(result.finish_reason, FinishReason::Completed));
    assert_eq!(result.exit_code, Some(0));

    // Store has the result keyed by the run id.
    let listed = harness.store.list(10).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].run_id, result.run_id);
}

#[tokio::test]
async fn callable_round_trip() {
    let dir = TempDir::new().unwrap();
    let ndjson = "{\"type\":\"result\",\"text\":\"done\"}\n";
    let harness = make_harness(MockVendor::new(ndjson));
    let req = CliRequest::new(
        CliVendorKind::Other("mock".into()),
        dir.path(),
        "ignored",
    );
    let v = serde_json::to_value(&req).unwrap();
    let out = harness.call(v, ctx()).await.unwrap();
    let parsed: atomr_agents_coding_cli_core::CliResult = serde_json::from_value(out).unwrap();
    assert_eq!(parsed.final_text, "done");
}

#[tokio::test]
async fn invalid_workdir_is_rejected() {
    let ndjson = "{\"type\":\"result\",\"text\":\"x\"}\n";
    let harness = make_harness(MockVendor::new(ndjson));
    let req = CliRequest::new(
        CliVendorKind::Other("mock".into()),
        "/this/path/does/not/exist",
        "ignored",
    );
    let err = harness.run(req).await.unwrap_err();
    assert!(matches!(
        err,
        atomr_agents_coding_cli_harness::HarnessError::InvalidWorkdir(_)
    ));
}

#[tokio::test]
async fn unknown_vendor_is_rejected() {
    let dir = TempDir::new().unwrap();
    let harness = CodingCliHarness::new(
        CodingCliHarnessSpec::default(),
        VendorRegistry::new(), // empty
        Arc::new(LocalIsolator::new()),
        Arc::new(InMemoryRunStore::new()),
    );
    let req = CliRequest::new(
        CliVendorKind::Other("nope".into()),
        dir.path(),
        "ignored",
    );
    let err = harness.run(req).await.unwrap_err();
    assert!(matches!(
        err,
        atomr_agents_coding_cli_harness::HarnessError::UnknownVendor(_)
    ));
}

#[tokio::test]
async fn interactive_mode_rejected_by_run() {
    let dir = TempDir::new().unwrap();
    let ndjson = "{\"type\":\"result\",\"text\":\"x\"}\n";
    let harness = make_harness(MockVendor::new(ndjson));
    let req = CliRequest::new(
        CliVendorKind::Other("mock".into()),
        dir.path(),
        "ignored",
    )
    .with_mode(RunMode::Interactive);
    let err = harness.run(req).await.unwrap_err();
    assert!(matches!(
        err,
        atomr_agents_coding_cli_harness::HarnessError::InvalidRequest(_)
    ));
}
