//! Citation-verifier role + deterministic default.

use async_trait::async_trait;
use atomr_agents_deep_research_core::{
    CitationStatus, CoverageSignals, NodeKind, NodeStep, SubQuestionStatus,
};

use crate::error::Result;
use crate::handle::ResearchHandle;

/// Role: validate citation markers, dedupe / renumber, and compute
/// coverage signals.
#[async_trait]
pub trait CitationVerifier: Send + Sync + 'static {
    async fn verify(&self, handle: &ResearchHandle) -> Result<()>;
}

#[async_trait]
impl CitationVerifier for Box<dyn CitationVerifier> {
    async fn verify(&self, handle: &ResearchHandle) -> Result<()> {
        (**self).verify(handle).await
    }
}

/// Deterministic, offline citation pass:
///
/// - Renumbers citations to be contiguous starting from 1.
/// - Marks them `Verified` (since the deterministic researcher only
///   records citations that actually came back from the provider).
/// - Recomputes coverage signals from the plan + draft.
#[derive(Default)]
pub struct DeterministicCitationVerifier;

impl DeterministicCitationVerifier {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl CitationVerifier for DeterministicCitationVerifier {
    async fn verify(&self, handle: &ResearchHandle) -> Result<()> {
        // Renumber + dedupe.
        handle.renumber_citations();
        let snap_before = handle.snapshot();
        let numbers: Vec<u32> = snap_before.citations.iter().map(|c| c.number).collect();
        for n in numbers {
            handle.mark_citation_status(n, CitationStatus::Verified);
        }

        // Coverage signals.
        let mut coverage = CoverageSignals::default();
        if let Some(plan) = &snap_before.plan {
            for sq in &plan.sub_questions {
                match sq.status {
                    SubQuestionStatus::Answered => coverage.sub_questions_answered += 1,
                    _ => coverage.sub_questions_unresolved += 1,
                }
            }
            // Section-level confidence: drafts with at least one
            // citation marker → 1.0; empty / no-marker → 0.0.
            let citation_re = regex::Regex::new(r"\[\d+\]").unwrap();
            for section in &snap_before.artifacts.drafts {
                let conf = if citation_re.is_match(&section.body) {
                    1.0
                } else {
                    0.0
                };
                if conf == 0.0 {
                    coverage.unresolved_gaps.push(section.heading.clone());
                }
                coverage
                    .confidence_per_section
                    .insert(section.heading.clone(), conf);
            }
        }
        handle.set_coverage(coverage);

        handle.push_transcript(NodeStep::new(
            NodeKind::Verifier,
            "verifier",
            format!(
                "verified {} citations; {} answered sub-questions",
                snap_before.citations.len(),
                handle.snapshot().coverage.sub_questions_answered
            ),
        ));

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_deep_research_core::{
        Citation, DraftSection, Plan, ResearchRequest, ResearchResult, SubQuestion,
    };
    use atomr_agents_web_search_core::MockWebSearch;
    use parking_lot::Mutex;
    use std::sync::Arc;
    use url::Url;

    #[tokio::test]
    async fn verifier_renumbers_and_marks_verified() {
        let req = ResearchRequest::new("q");
        let result = Arc::new(Mutex::new(ResearchResult::new(req.query.clone(), "test")));
        let h = ResearchHandle::new(result, Arc::new(req), Arc::new(MockWebSearch::new()));
        // Two citations sharing a URL — should dedupe.
        h.append_citation(Citation::new(
            5,
            Url::parse("https://a.test/").unwrap(),
            "A",
            "snippet",
        ));
        h.append_citation(Citation::new(
            9,
            Url::parse("https://a.test/").unwrap(),
            "A again",
            "snip",
        ));
        h.append_citation(Citation::new(
            11,
            Url::parse("https://b.test/").unwrap(),
            "B",
            "snippet b",
        ));

        let mut plan = Plan::new();
        let mut sq = SubQuestion::new("sq-1", "x");
        sq.status = SubQuestionStatus::Answered;
        plan.sub_questions.push(sq);
        h.set_plan(plan);
        h.append_draft_section(DraftSection {
            heading: "Findings".into(),
            body: "Note [1].".into(),
            answers_sub_questions: vec!["sq-1".into()],
        });

        DeterministicCitationVerifier.verify(&h).await.unwrap();
        let snap = h.snapshot();
        assert_eq!(snap.citations.len(), 2, "duplicate URL should be deduped");
        assert_eq!(snap.citations[0].number, 1);
        assert_eq!(snap.citations[1].number, 2);
        assert!(snap
            .citations
            .iter()
            .all(|c| c.status == CitationStatus::Verified));
        assert_eq!(snap.coverage.sub_questions_answered, 1);
        assert_eq!(snap.coverage.confidence_per_section.get("Findings"), Some(&1.0));
    }
}
