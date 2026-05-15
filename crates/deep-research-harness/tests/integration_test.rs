//! End-to-end integration tests for the three v1 strategies.
//!
//! All three exercise the same `ResearchRequest` → `ResearchResult`
//! contract against the deterministic LLM-free default roles + the
//! [`MockWebSearch`] provider. They differ in transcript shape, not
//! schema.

use std::sync::Arc;

use atomr_agents_deep_research_core::{ResearchRequest, ResearchState, SubQuestionStatus};
use atomr_agents_deep_research_harness::{
    ClarifyPlanSearchVerifyLoop, DeepResearchHarness, DeepResearchHarnessSpec, DeepResearchRoles,
    InMemoryResearchStore, IterationCapTermination, IterativeDeepeningLoop, MultiAgentParallelLoop,
};
use atomr_agents_web_search_core::{MockWebSearch, WebSearchHit};
use url::Url;

fn corpus() -> MockWebSearch {
    MockWebSearch::new()
        .with_fixture(
            "actor",
            vec![
                WebSearchHit::new(
                    Url::parse("https://docs.rs/actix").unwrap(),
                    "actix docs",
                    "actor framework for rust based on a message-passing runtime",
                ),
                WebSearchHit::new(
                    Url::parse("https://docs.rs/ractor").unwrap(),
                    "ractor docs",
                    "alternative actor framework, focused on supervision trees",
                ),
            ],
        )
        .with_fixture(
            "rust",
            vec![WebSearchHit::new(
                Url::parse("https://rust-lang.org/").unwrap(),
                "Rust homepage",
                "official rust language website",
            )],
        )
        .with_fixture(
            "compare",
            vec![WebSearchHit::new(
                Url::parse("https://compare.test/").unwrap(),
                "Comparison post",
                "comparison piece on rust frameworks",
            )],
        )
}

fn harness_clarify<L>(strategy: L) -> DeepResearchHarness<L, IterationCapTermination>
where
    L: atomr_agents_deep_research_harness::DeepResearchLoopStrategy,
{
    DeepResearchHarness::new(
        DeepResearchHarnessSpec::new("dr-test").with_max_iterations(64),
        Arc::new(InMemoryResearchStore::new()),
        Arc::new(corpus()),
        DeepResearchRoles::defaults(),
        strategy,
        IterationCapTermination::new(64),
    )
}

#[tokio::test]
async fn clarify_plan_search_verify_produces_final_report() {
    let harness = harness_clarify(ClarifyPlanSearchVerifyLoop::new());
    let req = ResearchRequest::new("compare actor frameworks in rust")
        .with_breadth(2)
        .with_depth(1);
    let result = harness.run(req).await.unwrap();

    assert_eq!(result.state, ResearchState::Done);
    assert_eq!(result.strategy, "clarify-plan-search-verify");
    assert!(result.final_report.is_some(), "final report missing");
    assert!(!result.citations.is_empty(), "no citations");
    assert!(result.plan.is_some(), "no plan");
    let plan = result.plan.unwrap();
    assert!(!plan.sub_questions.is_empty());
    assert!(plan
        .sub_questions
        .iter()
        .any(|s| s.status == SubQuestionStatus::Answered));
    assert!(result.transcript.iter().any(|s| s.label == "verifier"));
    assert!(result.coverage.sub_questions_answered > 0);
}

#[tokio::test]
async fn multi_agent_parallel_produces_final_report() {
    let harness = harness_clarify(MultiAgentParallelLoop::new());
    let req = ResearchRequest::new("compare actor frameworks in rust").with_breadth(2);
    let result = harness.run(req).await.unwrap();

    assert_eq!(result.state, ResearchState::Done);
    assert_eq!(result.strategy, "multi-agent-parallel");
    assert!(result.final_report.is_some());
    assert!(!result.citations.is_empty());
    // Parallel strategy should produce researcher transcript entries.
    let researcher_steps = result
        .transcript
        .iter()
        .filter(|s| s.label.starts_with("researcher:"))
        .count();
    assert!(
        researcher_steps >= 1,
        "expected parallel researcher entries, got transcript: {:#?}",
        result.transcript
    );
}

#[tokio::test]
async fn iterative_deepening_produces_final_report() {
    let harness = harness_clarify(IterativeDeepeningLoop::new());
    let req = ResearchRequest::new("compare actor frameworks in rust")
        .with_breadth(2)
        .with_depth(2);
    let result = harness.run(req).await.unwrap();

    assert_eq!(result.state, ResearchState::Done);
    assert_eq!(result.strategy, "iterative-deepening");
    assert!(result.final_report.is_some());
    // Critic should have fired at least once (the supervisor).
    let critic_count = result
        .transcript
        .iter()
        .filter(|s| s.role == atomr_agents_deep_research_core::NodeKind::Critic)
        .count();
    assert!(critic_count >= 1);
}

#[tokio::test]
async fn three_strategies_produce_same_schema() {
    // Same query → three results with the same TOP-LEVEL fields.
    let req_factory = || {
        ResearchRequest::new("compare actor frameworks")
            .with_breadth(2)
            .with_depth(1)
    };

    let a = harness_clarify(ClarifyPlanSearchVerifyLoop::new())
        .run(req_factory())
        .await
        .unwrap();
    let b = harness_clarify(MultiAgentParallelLoop::new())
        .run(req_factory())
        .await
        .unwrap();
    let c = harness_clarify(IterativeDeepeningLoop::new())
        .run(req_factory())
        .await
        .unwrap();

    for r in [&a, &b, &c] {
        assert_eq!(r.state, ResearchState::Done);
        assert!(r.final_report.is_some());
        assert!(r.plan.is_some());
        assert!(!r.id.is_empty());
        // citations may be empty if no fixture matched but the schema
        // must be present.
        let _ = serde_json::to_value(r).expect("must round-trip to JSON");
    }
    // Strategies must announce distinct names.
    assert_ne!(a.strategy, b.strategy);
    assert_ne!(b.strategy, c.strategy);
}

#[tokio::test]
async fn run_persists_each_iteration() {
    let store = Arc::new(InMemoryResearchStore::new());
    let harness = DeepResearchHarness::new(
        DeepResearchHarnessSpec::new("persist-test").with_max_iterations(64),
        store.clone(),
        Arc::new(corpus()),
        DeepResearchRoles::defaults(),
        ClarifyPlanSearchVerifyLoop::new(),
        IterationCapTermination::new(64),
    );
    let req = ResearchRequest::new("compare actor frameworks")
        .with_breadth(2)
        .with_depth(1);
    let result = harness.run(req).await.unwrap();

    // The store should now hold the final result.
    let loaded = atomr_agents_deep_research_harness::ResearchStore::get(&*store, &result.id)
        .await
        .unwrap()
        .expect("result should be persisted");
    assert_eq!(loaded.state, ResearchState::Done);
}
