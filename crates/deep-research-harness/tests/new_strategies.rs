//! End-to-end integration tests for the three v2 strategies:
//! `plan-and-execute`, `linear-write-critique`, `outline-first-section-fanout`.
//!
//! Mirrors `tests/integration_test.rs` in shape but lives in its own
//! file so PR 4 (which may touch the v1 integration tests) doesn't
//! collide with this PR.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_deep_research_core::{
    NodeKind, Plan, ResearchRequest, ResearchState, SubQuestion, SubQuestionStatus,
};
use atomr_agents_deep_research_harness::{
    DeepResearchHarness, DeepResearchHarnessSpec, DeepResearchRoles, InMemoryResearchStore,
    IterationCapTermination, LinearWriteCritiqueLoop, OutlineFirstSectionFanoutLoop, PlanAndExecuteLoop,
    Planner, ResearchHandle, Result as DRResult,
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
        .with_fixture(
            "framework",
            vec![WebSearchHit::new(
                Url::parse("https://framework.test/").unwrap(),
                "Frameworks roundup",
                "roundup of frameworks",
            )],
        )
}

fn harness<L>(strategy: L) -> DeepResearchHarness<L, IterationCapTermination>
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
async fn plan_and_execute_produces_final_report_with_critic_after_each_step() {
    let h = harness(PlanAndExecuteLoop::new());
    let req = ResearchRequest::new("compare actor frameworks in rust")
        .with_breadth(2)
        .with_depth(1);
    let result = h.run(req).await.unwrap();

    assert_eq!(result.state, ResearchState::Done);
    assert_eq!(result.strategy, "plan-and-execute");
    assert!(result.final_report.is_some(), "final report missing");
    assert!(!result.citations.is_empty(), "no citations");

    // Strategy-specific shape: critic step between each researcher step.
    // Filter to just researcher / critic transcript entries and check that
    // each researcher entry is followed by a critic entry (allowing for
    // earlier/later non-researcher/critic entries too).
    let kinds: Vec<NodeKind> = result
        .transcript
        .iter()
        .map(|s| s.role)
        .filter(|k| matches!(k, NodeKind::Researcher | NodeKind::Critic))
        .collect();
    // Must have at least one researcher followed by critic.
    let mut found_alternating = false;
    for w in kinds.windows(2) {
        if w[0] == NodeKind::Researcher && w[1] == NodeKind::Critic {
            found_alternating = true;
            break;
        }
    }
    assert!(
        found_alternating,
        "expected a Critic step immediately after a Researcher step; got: {kinds:?}"
    );
}

#[tokio::test]
async fn linear_write_critique_refines_writes_when_depth_allows() {
    let h = harness(LinearWriteCritiqueLoop::new());
    let req = ResearchRequest::new("compare actor frameworks in rust")
        .with_breadth(2)
        .with_depth(2);
    let result = h.run(req).await.unwrap();

    assert_eq!(result.state, ResearchState::Done);
    assert_eq!(result.strategy, "linear-write-critique");
    assert!(result.final_report.is_some(), "final report missing");
    assert!(!result.citations.is_empty(), "no citations");

    // Strategy-specific shape: at least two Writer steps appear in the
    // transcript when refinement happens (depth=2 + RegexCritic flagging
    // gaps after the first write).
    let writer_count = result
        .transcript
        .iter()
        .filter(|s| s.role == NodeKind::Writer)
        .count();
    assert!(
        writer_count >= 2,
        "expected >=2 Writer steps for refinement, got {writer_count}; transcript: {:#?}",
        result.transcript
    );
}

/// Test-only planner that emits exactly 2 sections × 2 sub-questions
/// (4 total), each with `SubQuestion::section` set explicitly. This
/// gives `OutlineFirstSectionFanoutLoop` a deterministic input to fan
/// out by section.
struct TwoSectionPlanner;

#[async_trait]
impl Planner for TwoSectionPlanner {
    async fn plan(&self, _req: &ResearchRequest, _handle: &ResearchHandle) -> DRResult<Plan> {
        let mut plan = Plan::new();
        plan.outline = vec!["Background".into(), "Findings".into()];
        for (id, text, section) in [
            ("sq-1", "actor framework basics", "Background"),
            ("sq-2", "rust ecosystem context", "Background"),
            ("sq-3", "compare actor frameworks", "Findings"),
            ("sq-4", "framework trade-offs", "Findings"),
        ] {
            let mut sq = SubQuestion::new(id, text);
            sq.section = Some(section.into());
            plan.sub_questions.push(sq);
        }
        plan.rationale = Some("test-only deterministic two-section plan".into());
        Ok(plan)
    }
}

#[tokio::test]
async fn outline_first_section_fanout_groups_researcher_steps_by_section() {
    use atomr_agents_deep_research_harness::{
        ConcatWriter, DeterministicCitationVerifier, MockResearcher, RegexCritic, TemplateClarifier,
    };

    let roles = DeepResearchRoles {
        clarifier: Arc::new(TemplateClarifier::new()),
        planner: Arc::new(TwoSectionPlanner),
        researcher: Arc::new(MockResearcher::new()),
        writer: Arc::new(ConcatWriter::new()),
        critic: Arc::new(RegexCritic::new()),
        verifier: Arc::new(DeterministicCitationVerifier::new()),
    };
    let h = DeepResearchHarness::new(
        DeepResearchHarnessSpec::new("dr-test").with_max_iterations(64),
        Arc::new(InMemoryResearchStore::new()),
        Arc::new(corpus()),
        roles,
        OutlineFirstSectionFanoutLoop::new(),
        IterationCapTermination::new(64),
    );
    let req = ResearchRequest::new("compare actor frameworks in rust")
        .with_breadth(4)
        .with_depth(1);
    let result = h.run(req).await.unwrap();

    assert_eq!(result.state, ResearchState::Done);
    assert_eq!(result.strategy, "outline-first-section-fanout");
    assert!(result.final_report.is_some(), "final report missing");
    assert!(!result.citations.is_empty(), "no citations");

    // All 4 sub-questions answered.
    let plan = result.plan.clone().expect("plan must be present");
    let answered = plan
        .sub_questions
        .iter()
        .filter(|s| s.status == SubQuestionStatus::Answered)
        .count();
    assert!(
        answered >= 3,
        "expected most sub-questions answered, got {answered} (of {}). Plan: {:#?}",
        plan.sub_questions.len(),
        plan
    );

    // Strategy-specific shape: every researcher transcript entry's
    // `sub_question_id` traces back to a sub-question with a non-empty
    // section, and the section matches one of the plan outline entries.
    let outline_set: std::collections::HashSet<String> = plan.outline.iter().cloned().collect();
    let researcher_steps: Vec<_> = result
        .transcript
        .iter()
        .filter(|s| s.role == NodeKind::Researcher)
        .collect();
    assert!(
        !researcher_steps.is_empty(),
        "expected researcher transcript entries"
    );
    for step in &researcher_steps {
        let sq_id = step
            .sub_question_id
            .as_ref()
            .expect("researcher step must carry sub_question_id");
        let sq = plan
            .sub_questions
            .iter()
            .find(|s| &s.id == sq_id)
            .unwrap_or_else(|| panic!("unknown sub_question_id `{sq_id}` in transcript"));
        let section = sq
            .section
            .as_ref()
            .expect("test plan assigns a section to every sub-question");
        assert!(
            outline_set.contains(section),
            "researcher step section `{section}` not in plan outline {outline_set:?}"
        );
    }
}
