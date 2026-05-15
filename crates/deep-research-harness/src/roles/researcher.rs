//! Researcher role + deterministic [`MockResearcher`] default.

use async_trait::async_trait;
use atomr_agents_deep_research_core::{Citation, NodeKind, NodeStep, SubQuestion, SubQuestionStatus};
use atomr_agents_web_search_core::WebSearchRequest;

use crate::error::Result;
use crate::handle::ResearchHandle;

/// Role: drive one sub-question to evidence + a compressed finding.
#[async_trait]
pub trait Researcher: Send + Sync + 'static {
    async fn research(&self, sub: &SubQuestion, handle: &ResearchHandle) -> Result<()>;
}

#[async_trait]
impl Researcher for Box<dyn Researcher> {
    async fn research(&self, sub: &SubQuestion, handle: &ResearchHandle) -> Result<()> {
        (**self).research(sub, handle).await
    }
}

/// Deterministic default. For each sub-question:
///
/// 1. Calls the configured [`WebSearch`](
///    atomr_agents_web_search_core::WebSearch) with `breadth` results.
/// 2. Records each raw hit in the artifacts.
/// 3. Appends one citation per hit.
/// 4. Marks the sub-question `Answered` (or `Unresolved` when zero hits).
#[derive(Default)]
pub struct MockResearcher;

impl MockResearcher {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Researcher for MockResearcher {
    async fn research(&self, sub: &SubQuestion, handle: &ResearchHandle) -> Result<()> {
        let request = handle.request();
        let mut search_req = WebSearchRequest::new(sub.text.clone()).with_max_results(request.breadth.max(1));
        search_req.allowed_domains = request.scope.allowed_domains.clone();
        search_req.blocked_domains = request.scope.blocked_domains.clone();
        let provider = handle.search();
        let provider_name = provider.provider_name().to_string();
        let hits = provider.search(&search_req).await?;

        for hit in &hits {
            handle.record_search_hit(provider_name.clone(), hit, Some(sub.id.clone()));
            let cite = Citation::new(0, hit.url.clone(), hit.title.clone(), hit.snippet.clone());
            handle.append_citation(Citation {
                supports: vec![sub.id.clone()],
                ..cite
            });
        }

        let status = if hits.is_empty() {
            SubQuestionStatus::Unresolved
        } else {
            SubQuestionStatus::Answered
        };
        handle.set_sub_question_status(&sub.id, status)?;
        handle.push_transcript(NodeStep {
            role: NodeKind::Researcher,
            label: format!("researcher:{}", sub.id),
            ts: chrono::Utc::now(),
            summary: format!(
                "{} hits via {} for sub-question `{}`",
                hits.len(),
                provider_name,
                sub.text
            ),
            sub_question_id: Some(sub.id.clone()),
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_deep_research_core::{ResearchRequest, ResearchResult};
    use atomr_agents_web_search_core::{MockWebSearch, WebSearchHit};
    use parking_lot::Mutex;
    use std::sync::Arc;
    use url::Url;

    fn make_handle(query: &str, search: MockWebSearch) -> (ResearchHandle, Arc<Mutex<ResearchResult>>) {
        let req = ResearchRequest::new(query);
        let result = Arc::new(Mutex::new(ResearchResult::new(req.query.clone(), "test")));
        let h = ResearchHandle::new(result.clone(), Arc::new(req), Arc::new(search));
        // Seed plan with one sub-question.
        h.append_sub_question(SubQuestion::new("sq-1", "tell me about rust actors"));
        (h, result)
    }

    #[tokio::test]
    async fn mock_researcher_records_hits_and_answers_sub_question() {
        let mock = MockWebSearch::new().with_fixture(
            "rust",
            vec![
                WebSearchHit::new(Url::parse("https://a.test/").unwrap(), "A", "snippet A"),
                WebSearchHit::new(Url::parse("https://b.test/").unwrap(), "B", "snippet B"),
            ],
        );
        let (h, _) = make_handle("tell me about rust actors", mock);
        let sub = h.snapshot().plan.unwrap().sub_questions[0].clone();
        MockResearcher.research(&sub, &h).await.unwrap();
        let snap = h.snapshot();
        assert_eq!(snap.artifacts.raw_search_hits.len(), 2);
        assert_eq!(snap.citations.len(), 2);
        assert_eq!(
            snap.plan.unwrap().sub_questions[0].status,
            SubQuestionStatus::Answered
        );
    }

    #[tokio::test]
    async fn mock_researcher_marks_unresolved_on_zero_hits() {
        let mock = MockWebSearch::new();
        let (h, _) = make_handle("anything", mock);
        let sub = h.snapshot().plan.unwrap().sub_questions[0].clone();
        MockResearcher.research(&sub, &h).await.unwrap();
        assert_eq!(
            h.snapshot().plan.unwrap().sub_questions[0].status,
            SubQuestionStatus::Unresolved
        );
    }
}
