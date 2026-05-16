//! Tests for the `AgentBased{Role}` impls.
//!
//! These tests drive each role through a scripted
//! [`MockInferenceClient`] — no real model is invoked. The mock either
//! returns a canned JSON blob (Pattern B) or stages a sequence of
//! tool-call turns followed by a stop turn (Pattern A).
//!
//! Only built with `--features agent`.

#![cfg(feature = "agent")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use atomr_agents_agent::{InferenceClient, TurnResult};
use atomr_agents_core::{
    AgentError, IterationBudget, MoneyBudget, Result as CoreResult, TimeBudget, TokenBudget,
};
use atomr_agents_deep_research_core::{
    Citation, CitationStatus, Plan, ResearchRequest, ResearchResult, SubQuestion, SubQuestionStatus,
};
use atomr_agents_deep_research_harness::{
    AgentBasedCitationVerifier, AgentBasedClarifier, AgentBasedCritic, AgentBasedPlanner,
    AgentBasedResearcher, AgentBasedWriter, CitationVerifier, Clarifier, ClarifyOutcome, Critic,
    InferenceClientFactory, Planner, ResearchHandle, Researcher, Writer,
};
use atomr_agents_tool::{ParsedToolCall, Provider};
use atomr_agents_web_search_core::{MockWebSearch, WebSearchHit};
use atomr_infer_core::batch::ExecuteBatch;
use atomr_infer_core::tokens::{FinishReason, TokenUsage};
use parking_lot::Mutex as PlMutex;
use url::Url;

/// Scripted turn emitted by [`MockInferenceClient`]. Each scripted turn
/// is consumed in FIFO order; if the agent makes more turns than the
/// script supplies, the mock returns an empty `Stop` turn so the
/// pipeline cleanly exits.
struct ScriptTurn {
    text: String,
    tool_calls: Vec<ParsedToolCall>,
    finish_reason: FinishReason,
}

impl ScriptTurn {
    fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
        }
    }
    fn tool_call(name: &str, args: serde_json::Value) -> Self {
        Self {
            text: String::new(),
            tool_calls: vec![ParsedToolCall {
                id: format!("call-{name}"),
                name: name.into(),
                arguments_raw: args.to_string(),
            }],
            finish_reason: FinishReason::ToolCalls,
        }
    }
}

struct MockInferenceClient {
    script: PlMutex<std::collections::VecDeque<ScriptTurn>>,
}

impl MockInferenceClient {
    fn new(script: Vec<ScriptTurn>) -> Self {
        Self {
            script: PlMutex::new(script.into()),
        }
    }
}

#[async_trait]
impl InferenceClient for MockInferenceClient {
    fn provider(&self) -> Provider {
        Provider::OpenAi
    }

    async fn run(&self, _batch: ExecuteBatch) -> CoreResult<TurnResult> {
        let turn = self.script.lock().pop_front().unwrap_or_else(|| ScriptTurn {
            text: String::new(),
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
        });
        Ok(TurnResult {
            text: turn.text,
            usage: TokenUsage::default(),
            finish_reason: Some(turn.finish_reason),
            tool_calls: turn.tool_calls,
        })
    }
}

struct ScriptFactory {
    /// One script per `build()` call, popped FIFO. The factory uses
    /// `Mutex` so a single `&self` can drain it across role calls.
    scripts: Mutex<Vec<Vec<ScriptTurn>>>,
}

impl ScriptFactory {
    fn new(scripts: Vec<Vec<ScriptTurn>>) -> Self {
        Self {
            scripts: Mutex::new(scripts),
        }
    }
    fn single(script: Vec<ScriptTurn>) -> Arc<dyn InferenceClientFactory> {
        Arc::new(ScriptFactory::new(vec![script]))
    }
}

impl InferenceClientFactory for ScriptFactory {
    fn build(&self, _model_id: &str) -> atomr_agents_deep_research_harness::Result<Arc<dyn InferenceClient>> {
        let mut g = self.scripts.lock().unwrap();
        let script = if g.is_empty() { vec![] } else { g.remove(0) };
        Ok(Arc::new(MockInferenceClient::new(script)))
    }
}

// ---------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------

fn handle(query: &str) -> (ResearchRequest, ResearchHandle) {
    let mut req = ResearchRequest::new(query);
    req.llm_overrides.default_model = Some("mock-model".into());
    let result = Arc::new(parking_lot::Mutex::new(ResearchResult::new(
        req.query.clone(),
        "test",
    )));
    let h = ResearchHandle::new(result, Arc::new(req.clone()), Arc::new(MockWebSearch::new()));
    (req, h)
}

#[allow(dead_code)]
fn budgets() -> (TokenBudget, TimeBudget, MoneyBudget, IterationBudget) {
    (
        TokenBudget::new(20_000),
        TimeBudget::new(Duration::from_secs(30)),
        MoneyBudget::from_usd(1.0),
        IterationBudget::new(20),
    )
}

// ---------------------------------------------------------------------
// Pattern B: clarifier / planner / critic / verifier
// ---------------------------------------------------------------------

#[tokio::test]
async fn agent_clarifier_parses_ready_status() {
    let (req, h) = handle("q");
    let factory = ScriptFactory::single(vec![ScriptTurn::text(r#"{"status":"ready"}"#)]);
    let out = AgentBasedClarifier::new(factory).clarify(&req, &h).await.unwrap();
    assert!(matches!(out, ClarifyOutcome::Ready));
}

#[tokio::test]
async fn agent_clarifier_parses_need_answers() {
    let (req, h) = handle("q");
    let factory = ScriptFactory::single(vec![ScriptTurn::text(
        r#"```json
{"status":"need_answers","questions":["a?","b?"]}
```"#,
    )]);
    let out = AgentBasedClarifier::new(factory).clarify(&req, &h).await.unwrap();
    match out {
        ClarifyOutcome::NeedAnswers { questions } => assert_eq!(questions, vec!["a?", "b?"]),
        _ => panic!("expected NeedAnswers"),
    }
}

#[tokio::test]
async fn agent_clarifier_records_presupplied_clarifications_without_llm() {
    let (mut req, h) = handle("q");
    req.clarifications
        .push(atomr_agents_deep_research_core::ClarificationTurn {
            question: "scope?".into(),
            answer: "broad".into(),
        });
    // Empty script: if the LLM is called we'll get an empty `text`
    // which will then fail JSON parse — the assertion catches it.
    let factory = ScriptFactory::single(vec![]);
    let out = AgentBasedClarifier::new(factory).clarify(&req, &h).await.unwrap();
    assert!(matches!(out, ClarifyOutcome::Ready));
    let transcript = h.snapshot().transcript;
    assert!(transcript.iter().any(|s| s.summary.contains("pre-supplied")));
}

#[tokio::test]
async fn agent_planner_parses_plan_json() {
    let (req, h) = handle("compare X vs Y");
    let plan_json = r#"{
        "outline": ["Background", "Findings"],
        "sub_questions": [
            {"id":"sq-1","text":"what is X?","section":"Background"},
            {"id":"sq-2","text":"what is Y?","section":"Findings"}
        ],
        "rationale": "fanout"
    }"#;
    let factory = ScriptFactory::single(vec![ScriptTurn::text(plan_json)]);
    let plan = AgentBasedPlanner::new(factory).plan(&req, &h).await.unwrap();
    assert_eq!(plan.outline.len(), 2);
    assert_eq!(plan.sub_questions.len(), 2);
    assert_eq!(plan.sub_questions[0].id, "sq-1");
}

#[tokio::test]
async fn agent_planner_errors_on_invalid_json() {
    let (req, h) = handle("q");
    let factory = ScriptFactory::single(vec![ScriptTurn::text("not json at all")]);
    let err = AgentBasedPlanner::new(factory).plan(&req, &h).await.unwrap_err();
    assert!(format!("{err}").contains("JSON"));
}

#[tokio::test]
async fn agent_critic_parses_critique_outcome() {
    let (_req, h) = handle("q");
    let factory = ScriptFactory::single(vec![ScriptTurn::text(
        r#"{"summary":"good","gaps":[],"done":true}"#,
    )]);
    let out = AgentBasedCritic::new(factory).critique(&h).await.unwrap();
    assert!(out.done);
    assert_eq!(out.summary, "good");
    assert!(out.gaps.is_empty());
    // Critique recorded into the handle's artifacts.
    let snap = h.snapshot();
    assert!(snap.transcript.iter().any(|s| s.summary.contains("critic")));
}

#[tokio::test]
async fn agent_verifier_applies_verdicts_and_coverage() {
    let (_req, h) = handle("q");
    // Seed two citations + a plan with one answered sub-question +
    // one draft section that contains a citation marker.
    h.append_citation(Citation::new(
        1,
        Url::parse("https://a.test/").unwrap(),
        "A",
        "snip",
    ));
    h.append_citation(Citation::new(
        2,
        Url::parse("https://b.test/").unwrap(),
        "B",
        "snip",
    ));
    let mut plan = Plan::new();
    plan.outline = vec!["Findings".into()];
    let mut sq = SubQuestion::new("sq-1", "x");
    sq.status = SubQuestionStatus::Answered;
    sq.section = Some("Findings".into());
    plan.sub_questions.push(sq);
    h.set_plan(plan);
    h.append_draft_section(atomr_agents_deep_research_core::DraftSection {
        heading: "Findings".into(),
        body: "Note [1]".into(),
        answers_sub_questions: vec!["sq-1".into()],
    });

    let factory = ScriptFactory::single(vec![ScriptTurn::text(
        r#"{"verdicts":[{"number":1,"status":"verified"},{"number":2,"status":"flagged"}]}"#,
    )]);
    AgentBasedCitationVerifier::new(factory).verify(&h).await.unwrap();
    let snap = h.snapshot();
    assert_eq!(snap.citations.len(), 2);
    assert_eq!(snap.citations[0].status, CitationStatus::Verified);
    assert_eq!(snap.citations[1].status, CitationStatus::Flagged);
    assert_eq!(snap.coverage.sub_questions_answered, 1);
    assert_eq!(snap.coverage.confidence_per_section.get("Findings"), Some(&1.0));
}

// ---------------------------------------------------------------------
// Pattern A: researcher / writer (tool-loop)
// ---------------------------------------------------------------------

#[tokio::test]
async fn agent_researcher_drives_tool_loop() {
    let (_req, h) = handle("anything about rust");
    // Seed a sub-question + a search fixture so web_search returns hits.
    let sub = SubQuestion::new("sq-1", "rust actor frameworks");
    h.append_sub_question(sub.clone());
    let provider = MockWebSearch::new().with_fixture(
        "rust",
        vec![WebSearchHit::new(
            Url::parse("https://a.test/").unwrap(),
            "Rust",
            "snip",
        )],
    );
    // Reseat the handle with the seeded provider so web_search returns
    // something useful.
    let req2 = (*h.request()).clone();
    let result = Arc::new(parking_lot::Mutex::new(ResearchResult::new(
        req2.query.clone(),
        "test",
    )));
    let h = ResearchHandle::new(result, Arc::new(req2), Arc::new(provider));
    h.append_sub_question(sub.clone());

    // Script: tool_call(web_search) → tool_call(record_search_hit) →
    // tool_call(append_citation) → tool_call(set_sub_question_status) → stop.
    let factory = ScriptFactory::single(vec![
        ScriptTurn::tool_call("web_search", serde_json::json!({"query": "rust"})),
        ScriptTurn::tool_call(
            "record_search_hit",
            serde_json::json!({
                "provider": "mock-search",
                "sub_question_id": "sq-1",
                "hit": {
                    "url": "https://a.test/",
                    "title": "A",
                    "snippet": "snip",
                    "source": "a.test"
                }
            }),
        ),
        ScriptTurn::tool_call(
            "append_citation",
            serde_json::json!({
                "citation": {
                    "number": 0,
                    "url": "https://a.test/",
                    "title": "A",
                    "snippet": "snip",
                    "source": "a.test",
                    "supports": ["sq-1"]
                }
            }),
        ),
        ScriptTurn::tool_call(
            "set_sub_question_status",
            serde_json::json!({"sub_question_id": "sq-1", "status": "answered"}),
        ),
        ScriptTurn::text("done"),
    ]);

    AgentBasedResearcher::new(factory)
        .with_max_tool_iterations(8)
        .research(&sub, &h)
        .await
        .unwrap();

    let snap = h.snapshot();
    assert_eq!(snap.citations.len(), 1);
    assert_eq!(snap.artifacts.raw_search_hits.len(), 1);
    let plan = snap.plan.unwrap();
    assert_eq!(
        plan.sub_questions.iter().find(|s| s.id == "sq-1").unwrap().status,
        SubQuestionStatus::Answered
    );
}

#[tokio::test]
async fn agent_writer_drives_draft_and_final_report_tool_calls() {
    let (_req, h) = handle("compare X vs Y");
    let mut plan = Plan::new();
    plan.outline = vec!["Background".into()];
    let mut sq = SubQuestion::new("sq-1", "x");
    sq.section = Some("Background".into());
    plan.sub_questions.push(sq);
    h.set_plan(plan.clone());
    h.append_citation(Citation {
        supports: vec!["sq-1".into()],
        ..Citation::new(1, Url::parse("https://a.test/").unwrap(), "A", "snip")
    });

    let factory = ScriptFactory::single(vec![
        ScriptTurn::tool_call(
            "append_draft_section",
            serde_json::json!({
                "section": {
                    "heading": "Background",
                    "body": "Body [1].",
                    "answers_sub_questions": ["sq-1"]
                }
            }),
        ),
        ScriptTurn::tool_call(
            "set_final_report",
            serde_json::json!({"markdown": "# Report\n\nBody [1]."}),
        ),
        ScriptTurn::text("done"),
    ]);

    AgentBasedWriter::new(factory)
        .with_max_tool_iterations(5)
        .write(&plan, &h)
        .await
        .unwrap();
    let snap = h.snapshot();
    assert_eq!(snap.artifacts.drafts.len(), 1);
    assert_eq!(snap.artifacts.drafts[0].heading, "Background");
    assert!(snap.final_report.is_some());
    assert!(snap.final_report.unwrap().contains("[1]"));
}

#[tokio::test]
async fn role_errors_when_no_model_configured() {
    // Build a request with empty llm_overrides + no with_model_id on
    // the role — should fail with a Config error.
    let req = ResearchRequest::new("q");
    let result = Arc::new(parking_lot::Mutex::new(ResearchResult::new(
        req.query.clone(),
        "test",
    )));
    let h = ResearchHandle::new(result, Arc::new(req.clone()), Arc::new(MockWebSearch::new()));
    let factory = ScriptFactory::single(vec![ScriptTurn::text("ignored")]);
    let err = AgentBasedPlanner::new(factory).plan(&req, &h).await.unwrap_err();
    assert!(
        format!("{err}").contains("no model id"),
        "expected Config error mentioning model id; got {err}"
    );
}

// Quiet `unused`: `AgentError` import is used to dispatch on test
// failure paths if/when needed.
#[allow(dead_code)]
fn _agent_error_typecheck(_e: AgentError) {}
