//! Integration tests for the domain tools exported by
//! `atomr-agents-deep-research-harness::tools`.
//!
//! Per-tool unit tests live alongside each tool in `src/tools/*.rs`.
//! These exercise the public-API contract: building a
//! [`ResearchToolSet`] from a [`ResearchHandle`], dispatching every
//! tool through the [`Tool`] trait, and verifying the handle reflects
//! the mutations.

use std::sync::Arc;
use std::time::Duration;

use atomr_agents_core::{
    AgentError, CallCtx, InvokeCtx, IterationBudget, MoneyBudget, TimeBudget, TokenBudget, Value,
};
use atomr_agents_deep_research_core::{ResearchRequest, ResearchResult, SubQuestion};
use atomr_agents_deep_research_harness::{
    AppendCitationTool, AppendDraftSectionTool, AppendSubQuestionTool, RecordClarificationTool,
    RecordCritiqueTool, RecordSearchHitTool, ResearchHandle, ResearchToolSet, SetFinalReportTool,
    SetPlanTool, SetSubQuestionStatusTool,
};
use atomr_agents_tool::Tool;
use atomr_agents_web_search_core::MockWebSearch;
use parking_lot::Mutex;
use serde_json::json;

fn ctx() -> InvokeCtx {
    InvokeCtx {
        call: CallCtx {
            agent_id: None,
            tokens: TokenBudget::new(1000),
            time: TimeBudget::new(Duration::from_secs(5)),
            money: MoneyBudget::from_usd(1.0),
            iterations: IterationBudget::new(10),
            trace: vec![],
        },
        tool_call_id: "t-1".into(),
        raw_args: Value::Null,
    }
}

fn handle(query: &str) -> ResearchHandle {
    let req = ResearchRequest::new(query);
    let result = Arc::new(Mutex::new(ResearchResult::new(req.query.clone(), "test")));
    ResearchHandle::new(result, Arc::new(req), Arc::new(MockWebSearch::new()))
}

#[tokio::test]
async fn research_tool_set_bundles_every_tool() {
    let h = handle("any");
    let set = ResearchToolSet::new(h);
    let tools = set.all();
    let names: Vec<&str> = tools.iter().map(|t| t.descriptor().name.as_str()).collect();
    assert!(names.contains(&"record_clarification"));
    assert!(names.contains(&"set_plan"));
    assert!(names.contains(&"append_sub_question"));
    assert!(names.contains(&"set_sub_question_status"));
    assert!(names.contains(&"record_search_hit"));
    assert!(names.contains(&"append_citation"));
    assert!(names.contains(&"append_draft_section"));
    assert!(names.contains(&"record_critique"));
    assert!(names.contains(&"set_final_report"));
    assert_eq!(tools.len(), 9);
}

#[tokio::test]
async fn tools_descriptors_are_namespaced_under_deep_research() {
    let h = handle("any");
    let set = ResearchToolSet::new(h);
    for t in set.all() {
        let id = t.descriptor().id.as_str();
        assert!(
            id.starts_with("deep_research."),
            "tool {} has unexpected id {id}",
            t.descriptor().name
        );
    }
}

#[tokio::test]
async fn end_to_end_tool_dispatch_populates_handle() {
    let h = handle("an example query");

    // 1. record_clarification
    let t = RecordClarificationTool::new(h.clone());
    t.invoke(json!({ "question": "scope?", "answer": "broad" }), &ctx())
        .await
        .unwrap();

    // 2. set_plan with two sub_questions
    let plan_json = json!({
        "plan": {
            "outline": ["Background", "Findings"],
            "sub_questions": [
                {"id": "sq-1", "text": "what frameworks?", "section": "Background"},
                {"id": "sq-2", "text": "how compare?",      "section": "Findings"}
            ],
            "rationale": "fan-out"
        }
    });
    let t = SetPlanTool::new(h.clone());
    t.invoke(plan_json, &ctx()).await.unwrap();

    // 3. append_sub_question
    let t = AppendSubQuestionTool::new(h.clone());
    t.invoke(
        json!({
            "sub_question": {"id": "sq-3", "text": "any others?", "section": "Findings"}
        }),
        &ctx(),
    )
    .await
    .unwrap();

    // 4. record_search_hit
    let t = RecordSearchHitTool::new(h.clone());
    t.invoke(
        json!({
            "provider": "test",
            "sub_question_id": "sq-1",
            "hit": {
                "url": "https://a.test/",
                "title": "A",
                "snippet": "snip a",
                "source": "a.test"
            }
        }),
        &ctx(),
    )
    .await
    .unwrap();

    // 5. append_citation
    let t = AppendCitationTool::new(h.clone());
    let out = t
        .invoke(
            json!({
                "citation": {
                    "number": 0,
                    "url": "https://a.test/",
                    "title": "A",
                    "snippet": "snip a",
                    "source": "a.test",
                    "supports": ["sq-1"]
                }
            }),
            &ctx(),
        )
        .await
        .unwrap();
    assert_eq!(out.get("number").and_then(|v| v.as_u64()), Some(1));

    // 6. set_sub_question_status — mark sq-1 answered
    let t = SetSubQuestionStatusTool::new(h.clone());
    t.invoke(json!({"sub_question_id": "sq-1", "status": "answered"}), &ctx())
        .await
        .unwrap();

    // 7. append_draft_section
    let t = AppendDraftSectionTool::new(h.clone());
    t.invoke(
        json!({
            "section": {
                "heading": "Background",
                "body": "Frameworks exist [1].",
                "answers_sub_questions": ["sq-1"]
            }
        }),
        &ctx(),
    )
    .await
    .unwrap();

    // 8. record_critique
    let t = RecordCritiqueTool::new(h.clone());
    t.invoke(json!({"summary": "looks fine", "gaps": []}), &ctx())
        .await
        .unwrap();

    // 9. set_final_report
    let t = SetFinalReportTool::new(h.clone());
    t.invoke(json!({"markdown": "# Report\n\nBody [1]."}), &ctx())
        .await
        .unwrap();

    // Verify aggregated state.
    let snap = h.snapshot();
    let plan = snap.plan.expect("plan set by tool");
    assert_eq!(plan.outline, vec!["Background".to_string(), "Findings".into()]);
    assert!(plan.sub_questions.iter().any(|sq| sq.id == "sq-3"));
    assert_eq!(snap.citations.len(), 1);
    assert_eq!(snap.citations[0].number, 1);
    assert_eq!(snap.artifacts.raw_search_hits.len(), 1);
    assert_eq!(snap.artifacts.drafts.len(), 1);
    assert_eq!(snap.artifacts.drafts[0].heading, "Background");
    assert!(snap.final_report.is_some());
}

#[tokio::test]
async fn tools_reject_malformed_arguments() {
    let h = handle("q");
    let cases: Vec<(Arc<dyn Tool>, Value)> = vec![
        (Arc::new(RecordClarificationTool::new(h.clone())), json!({})),
        (Arc::new(SetPlanTool::new(h.clone())), json!({})),
        (Arc::new(AppendSubQuestionTool::new(h.clone())), json!({})),
        (Arc::new(SetSubQuestionStatusTool::new(h.clone())), json!({})),
        (Arc::new(RecordSearchHitTool::new(h.clone())), json!({})),
        (Arc::new(AppendCitationTool::new(h.clone())), json!({})),
        (Arc::new(AppendDraftSectionTool::new(h.clone())), json!({})),
        (Arc::new(RecordCritiqueTool::new(h.clone())), json!({})),
        (Arc::new(SetFinalReportTool::new(h.clone())), json!({})),
    ];
    for (tool, args) in cases {
        let name = tool.descriptor().name.clone();
        let err = tool.invoke(args, &ctx()).await.unwrap_err();
        assert!(
            matches!(err, AgentError::Tool(_)),
            "tool {name} should have returned AgentError::Tool, got {err:?}"
        );
    }
}

#[tokio::test]
async fn append_sub_question_auto_initializes_plan() {
    // No prior set_plan call; the tool should create one.
    let h = handle("q");
    let t = AppendSubQuestionTool::new(h.clone());
    t.invoke(json!({"sub_question": {"id": "sq-1", "text": "x"}}), &ctx())
        .await
        .unwrap();
    let snap = h.snapshot();
    let plan = snap.plan.expect("plan auto-created");
    assert_eq!(plan.sub_questions.len(), 1);
    assert_eq!(plan.sub_questions[0].id, "sq-1");
}

#[tokio::test]
async fn set_sub_question_status_errors_on_unknown_id() {
    let h = handle("q");
    let plan_tool = SetPlanTool::new(h.clone());
    plan_tool
        .invoke(json!({"plan": {"outline": [], "sub_questions": []}}), &ctx())
        .await
        .unwrap();
    let tool = SetSubQuestionStatusTool::new(h.clone());
    let err = tool
        .invoke(json!({"sub_question_id": "ghost", "status": "answered"}), &ctx())
        .await
        .unwrap_err();
    assert!(matches!(err, AgentError::Tool(_)));
}

#[tokio::test]
async fn citation_number_autoincrements_across_calls() {
    let h = handle("q");
    let tool = AppendCitationTool::new(h.clone());
    for i in 0..3 {
        let out = tool
            .invoke(
                json!({
                    "citation": {
                        "number": 0,
                        "url": format!("https://example.test/{i}"),
                        "title": format!("T{i}"),
                        "snippet": "s",
                        "source": "example.test"
                    }
                }),
                &ctx(),
            )
            .await
            .unwrap();
        assert_eq!(out.get("number").and_then(|v| v.as_u64()), Some((i + 1) as u64));
    }
    assert_eq!(h.snapshot().citations.len(), 3);
}

// Tiny helper: prove the bundle integrates cleanly with `SubQuestion`
// constructed by callers.
#[tokio::test]
async fn sub_question_helper_compiles_against_tool_args() {
    let _sq = SubQuestion::new("sq-1", "x");
}
