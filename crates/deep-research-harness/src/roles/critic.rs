//! Critic role + deterministic [`RegexCritic`] default.

use async_trait::async_trait;
use atomr_agents_deep_research_core::{NodeKind, NodeStep, SubQuestionStatus};
use regex::Regex;

use crate::error::Result;
use crate::handle::ResearchHandle;

/// Outcome of one critique pass.
#[derive(Debug, Clone)]
pub struct CritiqueOutcome {
    /// Free-text summary recorded on the transcript.
    pub summary: String,
    /// Empty when there is nothing more to do; otherwise lists the
    /// sub-questions (or topics) the critic suggests revisiting.
    pub gaps: Vec<String>,
    /// `true` if the critic considers the draft good enough.
    pub done: bool,
}

/// Role: inspect the running draft + plan and report gaps.
#[async_trait]
pub trait Critic: Send + Sync + 'static {
    async fn critique(&self, handle: &ResearchHandle) -> Result<CritiqueOutcome>;
}

#[async_trait]
impl Critic for Box<dyn Critic> {
    async fn critique(&self, handle: &ResearchHandle) -> Result<CritiqueOutcome> {
        (**self).critique(handle).await
    }
}

/// Deterministic default. Flags:
///
/// - Sections in the draft that contain no `[n]` citation marker.
/// - Sub-questions still in `Pending` or `Unresolved` state.
/// - Duplicate citation URLs in the live citation list.
pub struct RegexCritic {
    citation_re: Regex,
}

impl Default for RegexCritic {
    fn default() -> Self {
        Self {
            citation_re: Regex::new(r"\[\d+\]").expect("static regex"),
        }
    }
}

impl RegexCritic {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Critic for RegexCritic {
    async fn critique(&self, handle: &ResearchHandle) -> Result<CritiqueOutcome> {
        let snap = handle.snapshot();
        let mut gaps: Vec<String> = Vec::new();

        // Draft sections without any [N] marker.
        for section in &snap.artifacts.drafts {
            if !self.citation_re.is_match(&section.body) {
                gaps.push(format!("section_no_citations:{}", section.heading));
            }
        }

        // Unresolved sub-questions.
        if let Some(plan) = &snap.plan {
            for sq in &plan.sub_questions {
                if matches!(
                    sq.status,
                    SubQuestionStatus::Pending | SubQuestionStatus::Unresolved
                ) {
                    gaps.push(format!("unresolved:{}", sq.id));
                }
            }
        }

        // Duplicate citation URLs.
        let mut seen: Vec<&url::Url> = Vec::new();
        for c in &snap.citations {
            if seen.iter().any(|u| u == &&c.url) {
                gaps.push(format!("duplicate_citation:{}", c.url));
            } else {
                seen.push(&c.url);
            }
        }

        let done = gaps.is_empty();
        let summary = if done {
            "no gaps detected".into()
        } else {
            format!("{} gaps detected", gaps.len())
        };

        handle.record_critique(summary.clone(), gaps.clone());
        handle.push_transcript(NodeStep::new(
            NodeKind::Critic,
            "critic",
            format!("critique: {summary}"),
        ));

        Ok(CritiqueOutcome { summary, gaps, done })
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

    fn handle() -> ResearchHandle {
        let req = ResearchRequest::new("q");
        let result = Arc::new(Mutex::new(ResearchResult::new(req.query.clone(), "test")));
        ResearchHandle::new(result, Arc::new(req), Arc::new(MockWebSearch::new()))
    }

    #[tokio::test]
    async fn critic_flags_uncited_sections() {
        let h = handle();
        h.append_draft_section(DraftSection {
            heading: "Background".into(),
            body: "Just words.".into(),
            answers_sub_questions: vec![],
        });
        let out = RegexCritic::new().critique(&h).await.unwrap();
        assert!(!out.done);
        assert!(out.gaps.iter().any(|g| g.starts_with("section_no_citations:")));
    }

    #[tokio::test]
    async fn critic_passes_when_clean() {
        let h = handle();
        let mut plan = Plan::new();
        let mut sq = SubQuestion::new("sq-1", "x");
        sq.status = SubQuestionStatus::Answered;
        plan.sub_questions.push(sq);
        h.set_plan(plan);
        h.append_citation(Citation::new(
            1,
            Url::parse("https://a.test/").unwrap(),
            "A",
            "snippet",
        ));
        h.append_draft_section(DraftSection {
            heading: "Findings".into(),
            body: "Important [1].".into(),
            answers_sub_questions: vec!["sq-1".into()],
        });
        let out = RegexCritic::new().critique(&h).await.unwrap();
        assert!(out.done, "expected critic to pass; gaps: {:?}", out.gaps);
    }
}
