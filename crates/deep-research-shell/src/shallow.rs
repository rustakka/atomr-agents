//! Shallow research path.
//!
//! When the [`IntentClassifier`](crate::IntentClassifier) routes a
//! request to [`ResearchTier::Shallow`](crate::ResearchTier), the shell
//! defers to a [`ShallowResearcher`] instead of the full deep harness.
//! The default [`DirectSearchShallow`] issues one [`WebSearch`] call
//! and synthesizes a [`ResearchResult`] directly — no clarifier, no
//! planner, no critic, no verify loop.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use atomr_agents_deep_research_core::{
    Citation, CitationStatus, NodeKind, NodeStep, Plan, RawSearchHit, ResearchRequest, ResearchResult,
    ResearchState, Telemetry,
};
use atomr_agents_web_search_core::{WebSearch, WebSearchRequest};
use chrono::Utc;

use crate::error::{Result, ShellError};

/// Object-safe trait for the shallow research path.
#[async_trait]
pub trait ShallowResearcher: Send + Sync + 'static {
    /// Produce a [`ResearchResult`] without engaging the deep harness.
    ///
    /// Implementations should set `result.strategy` to something
    /// descriptive so callers can tell shallow runs from deep runs
    /// downstream.
    async fn run(&self, req: &ResearchRequest) -> Result<ResearchResult>;
}

/// Default shallow researcher: one web-search call, results rendered as
/// a numbered markdown report.
///
/// This is intentionally non-LLM-driven so the shell can serve fast
/// queries without provider credentials. It mirrors the
/// `DeepResearchRoles::defaults()` philosophy: deterministic baseline,
/// callers swap in something smarter when they need it.
pub struct DirectSearchShallow {
    search: Arc<dyn WebSearch>,
    /// Floor on `max_results` per search. Defaults to `3`.
    pub min_results: u32,
    /// Provider label recorded against each `RawSearchHit` and used as
    /// the shallow `strategy` source tag in the transcript.
    pub source_label: String,
}

impl DirectSearchShallow {
    /// Wire a shallow researcher around an existing `WebSearch`
    /// implementation.
    pub fn new(search: Arc<dyn WebSearch>) -> Self {
        let label = search.provider_name().to_string();
        Self {
            search,
            min_results: 3,
            source_label: label,
        }
    }

    /// Override the `min_results` floor (the actual request uses
    /// `req.breadth.max(min_results)`).
    pub fn with_min_results(mut self, n: u32) -> Self {
        self.min_results = n;
        self
    }

    /// Override the provider label stamped onto raw hits.
    pub fn with_source_label(mut self, label: impl Into<String>) -> Self {
        self.source_label = label.into();
        self
    }
}

#[async_trait]
impl ShallowResearcher for DirectSearchShallow {
    async fn run(&self, req: &ResearchRequest) -> Result<ResearchResult> {
        let started = Instant::now();
        let max_results = req.breadth.max(self.min_results);
        let mut search_req = WebSearchRequest::new(req.query.clone()).with_max_results(max_results);
        if !req.scope.allowed_domains.is_empty() {
            search_req = search_req.with_allowed_domains(req.scope.allowed_domains.clone());
        }
        if !req.scope.blocked_domains.is_empty() {
            search_req.blocked_domains = req.scope.blocked_domains.clone();
        }

        let hits = self
            .search
            .search(&search_req)
            .await
            .map_err(ShellError::WebSearch)?;

        let now_ms = Utc::now().timestamp_millis();
        let mut result = ResearchResult {
            id: uuid::Uuid::new_v4().to_string(),
            query: req.query.clone(),
            strategy: "shallow-direct".to_string(),
            state: ResearchState::Done,
            final_report: None,
            citations: Vec::new(),
            plan: Some(Plan {
                outline: vec!["Summary".to_string()],
                sub_questions: Vec::new(),
                rationale: None,
            }),
            transcript: Vec::new(),
            coverage: Default::default(),
            telemetry: Telemetry::default(),
            artifacts: Default::default(),
            model_id: None,
            failure_reason: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        };

        // Record raw hits in artifacts (one per returned hit).
        for h in &hits {
            result.artifacts.raw_search_hits.push(RawSearchHit {
                provider: self.source_label.clone(),
                url: h.url.clone(),
                title: h.title.clone(),
                snippet: h.snippet.clone(),
                source: h.source.clone(),
                captured_at: Utc::now(),
                sub_question_id: None,
                content: h.content.clone(),
            });
        }

        // Build citations + final report.
        if hits.is_empty() {
            result.final_report = Some(format!("# {}\n\nNo results.\n", req.query));
        } else {
            let mut body = String::new();
            body.push_str(&format!("# {}\n\n", req.query));
            for (i, h) in hits.iter().enumerate() {
                let n = (i as u32) + 1;
                body.push_str(&format!("[{n}] **{}** — {}\n\n", h.title, h.snippet));
                let mut citation = Citation::new(n, h.url.clone(), h.title.clone(), h.snippet.clone());
                citation.source = h.source.clone();
                citation.published = h.published;
                citation.status = CitationStatus::Verified;
                result.citations.push(citation);
            }
            body.push_str("## References\n\n");
            for c in &result.citations {
                body.push_str(&format!("[{}] {}\n", c.number, c.url));
            }
            result.final_report = Some(body);
        }

        // Add a single transcript entry summarizing the shallow run.
        let summary = format!("Direct search returned {} hits", hits.len());
        result.transcript.push(NodeStep {
            role: NodeKind::Other,
            label: "shallow-direct".to_string(),
            ts: Utc::now(),
            summary,
            sub_question_id: None,
        });

        // Trivial telemetry: one tool call, measured wall time.
        let elapsed_ms = started.elapsed().as_millis() as u64;
        result.telemetry.tool_calls = 1;
        result.telemetry.wall_ms = elapsed_ms;

        let touch_ms = Utc::now().timestamp_millis();
        result.updated_at_ms = touch_ms;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_web_search_core::{MockWebSearch, WebSearchHit};
    use url::Url;

    fn hit(url: &str, title: &str) -> WebSearchHit {
        WebSearchHit::new(Url::parse(url).unwrap(), title, format!("snippet for {title}"))
    }

    #[tokio::test]
    async fn empty_results_produce_no_results_report() {
        let mock = Arc::new(MockWebSearch::new());
        let shallow = DirectSearchShallow::new(mock);
        let req = ResearchRequest::new("anything");
        let result = shallow.run(&req).await.unwrap();
        assert_eq!(result.strategy, "shallow-direct");
        assert_eq!(result.state, ResearchState::Done);
        assert!(result.citations.is_empty());
        assert!(result.final_report.as_deref().unwrap().contains("No results"));
    }

    #[tokio::test]
    async fn results_become_numbered_citations() {
        let mock = MockWebSearch::new().with_fixture(
            "rust",
            vec![
                hit("https://rust-lang.org/", "Rust"),
                hit("https://blog.rust-lang.org/", "Blog"),
            ],
        );
        let shallow = DirectSearchShallow::new(Arc::new(mock));
        let req = ResearchRequest::new("rust language");
        let result = shallow.run(&req).await.unwrap();
        assert_eq!(result.citations.len(), 2);
        assert_eq!(result.citations[0].number, 1);
        assert_eq!(result.citations[1].number, 2);
        let report = result.final_report.unwrap();
        assert!(report.contains("[1]"));
        assert!(report.contains("[2]"));
        assert!(report.contains("## References"));
        assert_eq!(result.telemetry.tool_calls, 1);
        assert_eq!(result.artifacts.raw_search_hits.len(), 2);
    }
}
