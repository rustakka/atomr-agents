//! Clarifier role + deterministic [`TemplateClarifier`] default.

use async_trait::async_trait;
use atomr_agents_deep_research_core::{HitlPolicy, NodeKind, NodeStep, ResearchRequest};

use crate::error::Result;
use crate::handle::ResearchHandle;

/// Outcome of a clarification step.
#[derive(Debug, Clone)]
pub enum ClarifyOutcome {
    /// No clarifications needed (or auto-answered).
    Ready,
    /// Stop and wait for the user to answer these questions.
    NeedAnswers { questions: Vec<String> },
}

/// Role: ask follow-up questions before the planner runs.
#[async_trait]
pub trait Clarifier: Send + Sync + 'static {
    async fn clarify(&self, req: &ResearchRequest, handle: &ResearchHandle) -> Result<ClarifyOutcome>;
}

#[async_trait]
impl Clarifier for Box<dyn Clarifier> {
    async fn clarify(&self, req: &ResearchRequest, handle: &ResearchHandle) -> Result<ClarifyOutcome> {
        (**self).clarify(req, handle).await
    }
}

/// Deterministic default. Detects ambiguity markers in the query
/// (`vs`, `compare`, `should I`, `which`, …) and proposes a small,
/// fixed list of clarifying questions. Under
/// [`HitlPolicy::AutoClarify`] (the default) it answers them from the
/// request scope and returns [`ClarifyOutcome::Ready`] so the run can
/// proceed without human input.
#[derive(Default)]
pub struct TemplateClarifier;

impl TemplateClarifier {
    pub fn new() -> Self {
        Self
    }

    /// Heuristic list of clarifying questions for the given query.
    pub fn questions_for(query: &str) -> Vec<String> {
        let mut qs = Vec::new();
        let lower = query.to_lowercase();
        if lower.contains(" vs ") || lower.contains("compare") {
            qs.push("Which dimensions matter most for the comparison (performance, ergonomics, ecosystem, license, …)?".into());
        }
        if lower.contains("best") || lower.contains("recommend") {
            qs.push("What are the constraints (budget, deployment environment, team size, latency)?".into());
        }
        if lower.contains("how") {
            qs.push(
                "Is the goal a step-by-step guide, an architecture sketch, or a survey of approaches?".into(),
            );
        }
        if qs.is_empty() {
            qs.push("Any preferred scope, sources, or time horizon for this question?".into());
        }
        qs
    }
}

#[async_trait]
impl Clarifier for TemplateClarifier {
    async fn clarify(&self, req: &ResearchRequest, handle: &ResearchHandle) -> Result<ClarifyOutcome> {
        // Honor pre-supplied clarifications.
        if !req.clarifications.is_empty() {
            for t in &req.clarifications {
                handle.record_clarification(t.question.clone(), t.answer.clone());
            }
            handle.push_transcript(NodeStep::new(
                NodeKind::Clarifier,
                "clarifier",
                format!(
                    "accepted {} pre-supplied clarifications",
                    req.clarifications.len()
                ),
            ));
            return Ok(ClarifyOutcome::Ready);
        }

        let questions = Self::questions_for(&req.query);
        match req.human_in_the_loop {
            HitlPolicy::AskOnce | HitlPolicy::AskEveryRound => {
                handle.push_transcript(NodeStep::new(
                    NodeKind::Clarifier,
                    "clarifier",
                    format!("requesting {} clarifications", questions.len()),
                ));
                Ok(ClarifyOutcome::NeedAnswers { questions })
            }
            HitlPolicy::AutoClarify | HitlPolicy::Off => {
                // Auto-answer from the scope so downstream roles always
                // see *some* context, even if synthetic.
                let scope = &req.scope;
                let auto_answer = if let Some(bg) = &scope.background {
                    bg.clone()
                } else if !scope.allowed_domains.is_empty() {
                    format!("Restrict to domains: {}", scope.allowed_domains.join(", "))
                } else {
                    "No further constraints — proceed with broad sources.".into()
                };
                for q in &questions {
                    handle.record_clarification(q.clone(), auto_answer.clone());
                }
                handle.push_transcript(NodeStep::new(
                    NodeKind::Clarifier,
                    "clarifier",
                    format!("auto-answered {} clarifications", questions.len()),
                ));
                Ok(ClarifyOutcome::Ready)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_deep_research_core::{ResearchRequest, ResearchResult};
    use atomr_agents_web_search_core::MockWebSearch;
    use parking_lot::Mutex;
    use std::sync::Arc;

    fn handle(req: &ResearchRequest) -> ResearchHandle {
        let result = Arc::new(Mutex::new(ResearchResult::new(req.query.clone(), "test")));
        ResearchHandle::new(result, Arc::new(req.clone()), Arc::new(MockWebSearch::new()))
    }

    #[tokio::test]
    async fn template_clarifier_auto_answers() {
        let req = ResearchRequest::new("compare actor frameworks in rust");
        let h = handle(&req);
        let out = TemplateClarifier.clarify(&req, &h).await.unwrap();
        assert!(matches!(out, ClarifyOutcome::Ready));
        assert!(h
            .snapshot()
            .transcript
            .iter()
            .any(|s| s.role == NodeKind::Clarifier));
    }

    #[tokio::test]
    async fn template_clarifier_asks_when_policy_requires() {
        let mut req = ResearchRequest::new("compare frameworks");
        req.human_in_the_loop = HitlPolicy::AskOnce;
        let h = handle(&req);
        let out = TemplateClarifier.clarify(&req, &h).await.unwrap();
        match out {
            ClarifyOutcome::NeedAnswers { questions } => assert!(!questions.is_empty()),
            _ => panic!("expected NeedAnswers"),
        }
    }
}
