//! End-to-end integration tests for the two-tier shell.
//!
//! Covers four things:
//!
//! 1. The heuristic classifier verdict over a table of representative
//!    queries.
//! 2. The shallow path produces a well-shaped `ResearchResult`.
//! 3. The shallow path handles "no results" gracefully.
//! 4. The full shell routes between shallow and deep based on the
//!    classifier verdict.

use std::sync::Arc;

use atomr_agents_callable::Callable;
use atomr_agents_core::{CallCtx, HarnessId, IterationBudget, MoneyBudget, TimeBudget, TokenBudget};
use atomr_agents_deep_research_core::{ResearchRequest, ResearchResult, ResearchState};
use atomr_agents_deep_research_harness::{
    ClarifyPlanSearchVerifyLoop, DeepResearchHarness, DeepResearchHarnessRef, DeepResearchHarnessSpec,
    DeepResearchRoles, InMemoryResearchStore, IterationCapTermination,
};
use atomr_agents_deep_research_shell::{
    DeepResearchShell, DirectSearchShallow, HeuristicIntentClassifier, IntentClassifier, ResearchTier,
    ShallowResearcher,
};
use atomr_agents_web_search_core::{MockWebSearch, WebSearchHit};
use std::time::Duration;
use url::Url;

fn ctx() -> CallCtx {
    CallCtx {
        agent_id: None,
        tokens: TokenBudget::new(10_000),
        time: TimeBudget::new(Duration::from_secs(30)),
        money: MoneyBudget::from_usd(1.0),
        iterations: IterationBudget::new(64),
        trace: vec![],
    }
}

fn corpus() -> MockWebSearch {
    MockWebSearch::new()
        .with_fixture(
            "rust",
            vec![
                WebSearchHit::new(
                    Url::parse("https://rust-lang.org/").unwrap(),
                    "Rust homepage",
                    "official rust language website",
                ),
                WebSearchHit::new(
                    Url::parse("https://blog.rust-lang.org/").unwrap(),
                    "Rust blog",
                    "rust language blog",
                ),
            ],
        )
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
            "compare",
            vec![WebSearchHit::new(
                Url::parse("https://compare.test/").unwrap(),
                "Comparison",
                "comparison piece on rust frameworks",
            )],
        )
        .with_fixture(
            "tokio",
            vec![WebSearchHit::new(
                Url::parse("https://tokio.rs/").unwrap(),
                "Tokio runtime",
                "async runtime for rust",
            )],
        )
}

fn build_deep_ref(web: Arc<MockWebSearch>) -> DeepResearchHarnessRef {
    let harness = DeepResearchHarness::new(
        DeepResearchHarnessSpec::new("dr-shell-test").with_max_iterations(64),
        Arc::new(InMemoryResearchStore::new()),
        web,
        DeepResearchRoles::defaults(),
        ClarifyPlanSearchVerifyLoop::new(),
        IterationCapTermination::new(64),
    );
    DeepResearchHarnessRef::new(HarnessId::from("dr-shell-test"), Arc::new(harness.into_boxed()))
}

#[tokio::test]
async fn heuristic_classifier_table() {
    let classifier = HeuristicIntentClassifier::new();
    let cases: Vec<(&str, u32, ResearchTier)> = vec![
        ("rust", 1, ResearchTier::Shallow),
        ("compare actor frameworks in rust", 2, ResearchTier::Deep),
        ("how to install rustc", 0, ResearchTier::Shallow),
        (
            "what are the trade-offs between Tokio and async-std?",
            2,
            ResearchTier::Deep,
        ),
        ("rust", 3, ResearchTier::Deep),
    ];
    for (query, depth, expected) in cases {
        let req = ResearchRequest::new(query).with_depth(depth);
        let got = classifier.classify(&req).await.unwrap();
        assert_eq!(
            got, expected,
            "mismatch for query={query:?} depth={depth} -> expected {expected:?}, got {got:?}"
        );
    }
}

#[tokio::test]
async fn shallow_path_produces_valid_result() {
    let web = Arc::new(corpus());
    let shallow = DirectSearchShallow::new(web);
    let req = ResearchRequest::new("rust").with_breadth(3);
    let result = shallow.run(&req).await.unwrap();
    assert_eq!(result.strategy, "shallow-direct");
    assert_eq!(result.state, ResearchState::Done);
    assert!(result.final_report.is_some());
    assert!(!result.citations.is_empty());
    let plan = result.plan.expect("plan should be present");
    assert_eq!(plan.outline, vec!["Summary".to_string()]);
}

#[tokio::test]
async fn shallow_path_on_empty_results() {
    let web = Arc::new(MockWebSearch::new());
    let shallow = DirectSearchShallow::new(web);
    let req = ResearchRequest::new("zzzz-nonexistent-needle");
    let result = shallow.run(&req).await.unwrap();
    assert_eq!(result.strategy, "shallow-direct");
    assert_eq!(result.state, ResearchState::Done);
    assert!(result.citations.is_empty());
    let report = result.final_report.expect("report should be present");
    assert!(
        report.contains("No results"),
        "expected `No results` in report: {report}"
    );
}

#[tokio::test]
async fn shell_routes_short_query_to_shallow() {
    let web = Arc::new(corpus());
    let deep = build_deep_ref(web.clone());
    let shell = DeepResearchShell::new(
        Arc::new(HeuristicIntentClassifier::new()),
        Arc::new(DirectSearchShallow::new(web)),
        deep,
    );
    // `depth: 1` keeps it inside the shallow heuristic threshold;
    // `ResearchRequest`'s serde default is 2.
    let req = ResearchRequest::new("rust").with_depth(1);
    let v = shell
        .call(serde_json::to_value(&req).unwrap(), ctx())
        .await
        .unwrap();
    let result: ResearchResult = serde_json::from_value(v).unwrap();
    assert_eq!(result.strategy, "shallow-direct");
    assert_eq!(result.state, ResearchState::Done);
    assert!(!result.citations.is_empty());
}

#[tokio::test]
async fn shell_routes_long_comparative_query_to_deep() {
    let web = Arc::new(corpus());
    let deep = build_deep_ref(web.clone());
    let shell = DeepResearchShell::new(
        Arc::new(HeuristicIntentClassifier::new()),
        Arc::new(DirectSearchShallow::new(web)),
        deep,
    );
    let req = ResearchRequest::new("compare actor frameworks in rust")
        .with_breadth(2)
        .with_depth(1);
    let v = shell
        .call(serde_json::to_value(&req).unwrap(), ctx())
        .await
        .unwrap();
    let result: ResearchResult = serde_json::from_value(v).unwrap();
    assert_eq!(result.strategy, "clarify-plan-search-verify");
    assert_eq!(result.state, ResearchState::Done);
}
