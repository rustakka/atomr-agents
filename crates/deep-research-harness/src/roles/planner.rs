//! Planner role + deterministic [`HeuristicPlanner`] default.

use async_trait::async_trait;
use atomr_agents_deep_research_core::{NodeKind, NodeStep, Plan, ResearchRequest, SubQuestion};

use crate::error::Result;
use crate::handle::ResearchHandle;

/// Role: turn a query (plus clarifications) into an outline + sub-questions.
#[async_trait]
pub trait Planner: Send + Sync + 'static {
    async fn plan(&self, req: &ResearchRequest, handle: &ResearchHandle) -> Result<Plan>;
}

#[async_trait]
impl Planner for Box<dyn Planner> {
    async fn plan(&self, req: &ResearchRequest, handle: &ResearchHandle) -> Result<Plan> {
        (**self).plan(req, handle).await
    }
}

/// Deterministic default. Splits the query on sentence and conjunction
/// boundaries and assigns each fragment as a sub-question. Produces a
/// three-section outline (`Background`, `Findings`, `Conclusion`).
///
/// Resulting plans are deterministic given the same query — useful for
/// tests, mocks, and the web UI demo flow.
#[derive(Default)]
pub struct HeuristicPlanner;

impl HeuristicPlanner {
    pub fn new() -> Self {
        Self
    }

    fn sub_questions(query: &str, breadth: u32) -> Vec<SubQuestion> {
        // Split on `?`, `.`, `;`, ` and `, ` vs `.
        let mut chunks: Vec<String> = Vec::new();
        for part in query
            .split(['?', '.', ';', '\n'])
            .flat_map(|p| p.split(" and "))
            .flat_map(|p| p.split(" vs "))
        {
            let t = part.trim();
            if !t.is_empty() {
                chunks.push(t.to_string());
            }
        }
        if chunks.is_empty() {
            chunks.push(query.trim().to_string());
        }

        let take = breadth.max(1) as usize;
        chunks
            .into_iter()
            .take(take)
            .enumerate()
            .map(|(i, text)| {
                let mut sq = SubQuestion::new(format!("sq-{}", i + 1), text);
                sq.section = Some(match i {
                    0 => "Background".into(),
                    1 => "Findings".into(),
                    _ => "Conclusion".into(),
                });
                sq
            })
            .collect()
    }
}

#[async_trait]
impl Planner for HeuristicPlanner {
    async fn plan(&self, req: &ResearchRequest, handle: &ResearchHandle) -> Result<Plan> {
        let outline = vec!["Background".into(), "Findings".into(), "Conclusion".into()];
        let sub_questions = Self::sub_questions(&req.query, req.breadth);
        let plan = Plan {
            outline,
            sub_questions,
            rationale: Some("Heuristic split of the query on sentence/conjunction boundaries.".into()),
        };
        handle.push_transcript(NodeStep::new(
            NodeKind::Planner,
            "planner",
            format!("composed plan with {} sub-questions", plan.sub_questions.len()),
        ));
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_deep_research_core::ResearchResult;
    use atomr_agents_web_search_core::MockWebSearch;
    use parking_lot::Mutex;
    use std::sync::Arc;

    #[tokio::test]
    async fn heuristic_planner_splits_query() {
        let req = ResearchRequest::new("compare actor frameworks in rust vs go").with_breadth(3);
        let result = Arc::new(Mutex::new(ResearchResult::new(req.query.clone(), "test")));
        let h = ResearchHandle::new(result, Arc::new(req.clone()), Arc::new(MockWebSearch::new()));
        let plan = HeuristicPlanner.plan(&req, &h).await.unwrap();
        assert_eq!(plan.outline.len(), 3);
        assert!(!plan.sub_questions.is_empty());
        assert!(plan.sub_questions.len() <= 3);
    }

    #[tokio::test]
    async fn planner_respects_breadth_cap() {
        let mut req = ResearchRequest::new("a. b. c. d. e. f. g.");
        req.breadth = 2;
        let result = Arc::new(Mutex::new(ResearchResult::new(req.query.clone(), "test")));
        let h = ResearchHandle::new(result, Arc::new(req.clone()), Arc::new(MockWebSearch::new()));
        let plan = HeuristicPlanner.plan(&req, &h).await.unwrap();
        assert_eq!(plan.sub_questions.len(), 2);
    }
}
