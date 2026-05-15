//! Shared mutable handle for the in-flight [`ResearchResult`].
//!
//! Every role and every tool calls into this handle to mutate the
//! result. Mirrors `ToolHandle` from `meetings-harness`.

use std::sync::Arc;

use atomr_agents_deep_research_core::{
    Artifacts, Citation, CitationStatus, CoverageSignals, DraftSection, NodeKind, NodeStep, Plan,
    RawSearchHit, ResearchRequest, ResearchResult, ResearchState, SubQuestion, SubQuestionStatus, Telemetry,
};
use atomr_agents_retriever::Retriever;
use atomr_agents_web_search_core::{WebSearch, WebSearchHit};
use chrono::Utc;
use parking_lot::Mutex;
use tokio::sync::broadcast;
use url::Url;

use crate::error::{DeepResearchError, Result};
use crate::events::DeepResearchEvent;

/// Shared, cloneable handle that mutates the in-flight result.
#[derive(Clone)]
pub struct ResearchHandle {
    inner: Arc<Mutex<ResearchResult>>,
    request: Arc<ResearchRequest>,
    search: Arc<dyn WebSearch>,
    retriever: Option<Arc<dyn Retriever>>,
    events: Option<broadcast::Sender<DeepResearchEvent>>,
}

impl ResearchHandle {
    pub fn new(
        result: Arc<Mutex<ResearchResult>>,
        request: Arc<ResearchRequest>,
        search: Arc<dyn WebSearch>,
    ) -> Self {
        Self {
            inner: result,
            request,
            search,
            retriever: None,
            events: None,
        }
    }

    pub fn with_retriever(mut self, retriever: Arc<dyn Retriever>) -> Self {
        self.retriever = Some(retriever);
        self
    }

    pub fn with_events(mut self, sink: broadcast::Sender<DeepResearchEvent>) -> Self {
        self.events = Some(sink);
        self
    }

    /// Read-only access to the originating request.
    pub fn request(&self) -> Arc<ResearchRequest> {
        self.request.clone()
    }

    /// Access the configured web-search provider.
    pub fn search(&self) -> Arc<dyn WebSearch> {
        self.search.clone()
    }

    /// Access the optional local retriever.
    pub fn retriever(&self) -> Option<Arc<dyn Retriever>> {
        self.retriever.clone()
    }

    fn emit(&self, ev: DeepResearchEvent) {
        if let Some(tx) = &self.events {
            let _ = tx.send(ev);
        }
    }

    /// Read a snapshot of the in-flight result.
    pub fn snapshot(&self) -> ResearchResult {
        self.inner.lock().clone()
    }

    /// Update the in-flight result via a closure.
    pub fn with_result<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut ResearchResult) -> R,
    {
        let mut g = self.inner.lock();
        let out = f(&mut g);
        g.touch();
        out
    }

    /// Promote `state` to `Clarifying`.
    pub fn set_state(&self, state: ResearchState) {
        self.with_result(|r| r.state = state);
    }

    /// Record a clarification turn.
    pub fn record_clarification(&self, question: String, answer: String) {
        let count = self.with_result(|r| {
            r.transcript.push(NodeStep::new(
                NodeKind::Clarifier,
                "clarifier",
                format!("Q: {question} | A: {answer}"),
            ));
            r.transcript.len()
        });
        self.emit(DeepResearchEvent::ClarificationsRecorded { count });
    }

    /// Set / replace the plan.
    pub fn set_plan(&self, plan: Plan) {
        self.with_result(|r| {
            r.plan = Some(plan.clone());
            r.transcript.push(NodeStep::new(
                NodeKind::Planner,
                "planner",
                format!(
                    "plan: {} sub-questions, {} sections",
                    plan.sub_questions.len(),
                    plan.outline.len()
                ),
            ));
        });
        let snap_plan = self.snapshot().plan.unwrap_or_default();
        self.emit(DeepResearchEvent::PlanComposed { plan: snap_plan });
    }

    /// Append a sub-question to the plan; returns its id.
    pub fn append_sub_question(&self, sub: SubQuestion) -> String {
        let id = sub.id.clone();
        let sub_clone = sub.clone();
        self.with_result(|r| {
            r.plan.get_or_insert_with(Plan::new).sub_questions.push(sub);
        });
        self.emit(DeepResearchEvent::SubQuestionStarted {
            sub_question: sub_clone,
        });
        id
    }

    /// Update a sub-question's status.
    pub fn set_sub_question_status(&self, id: &str, status: SubQuestionStatus) -> Result<()> {
        let updated = self.with_result(|r| {
            let Some(plan) = r.plan.as_mut() else {
                return false;
            };
            let Some(s) = plan.sub_question_mut(id) else {
                return false;
            };
            s.status = status;
            true
        });
        if !updated {
            return Err(DeepResearchError::tool(format!("unknown sub_question_id `{id}`")));
        }
        Ok(())
    }

    /// Record a raw search hit in the artifacts.
    pub fn record_search_hit(
        &self,
        provider: impl Into<String>,
        hit: &WebSearchHit,
        sub_question_id: Option<String>,
    ) {
        let raw = RawSearchHit {
            provider: provider.into(),
            url: hit.url.clone(),
            title: hit.title.clone(),
            snippet: hit.snippet.clone(),
            source: hit.source.clone(),
            captured_at: Utc::now(),
            sub_question_id,
            content: hit.content.clone(),
        };
        let ev_provider = raw.provider.clone();
        let ev_url = raw.url.to_string();
        let ev_title = raw.title.clone();
        self.with_result(|r| r.artifacts.raw_search_hits.push(raw));
        self.emit(DeepResearchEvent::SearchHitRecorded {
            provider: ev_provider,
            url: ev_url,
            title: ev_title,
        });
    }

    /// Append a draft section.
    pub fn append_draft_section(&self, section: DraftSection) {
        let heading = section.heading.clone();
        let body_chars = section.body.chars().count();
        self.with_result(|r| {
            r.artifacts.drafts.push(section);
            r.transcript.push(NodeStep::new(
                NodeKind::Writer,
                "writer",
                format!("appended section `{heading}` ({body_chars} chars)"),
            ));
        });
        self.emit(DeepResearchEvent::DraftSectionAppended { heading, body_chars });
    }

    /// Append a citation. Auto-renumbers if `citation.number == 0`.
    pub fn append_citation(&self, mut citation: Citation) -> u32 {
        let number = self.with_result(|r| {
            if citation.number == 0 {
                citation.number = (r.citations.len() as u32) + 1;
            }
            let n = citation.number;
            r.citations.push(citation.clone());
            n
        });
        let final_cite = self.with_result(|r| r.citations.last().cloned()).unwrap();
        self.emit(DeepResearchEvent::CitationAppended { citation: final_cite });
        number
    }

    /// Set the final report body.
    pub fn set_final_report(&self, markdown: String) {
        self.with_result(|r| {
            r.final_report = Some(markdown);
        });
    }

    /// Record a critic step.
    pub fn record_critique(&self, summary: String, gaps: Vec<String>) {
        let summary_clone = summary.clone();
        let gaps_clone = gaps.clone();
        self.with_result(|r| {
            r.transcript.push(NodeStep::new(
                NodeKind::Critic,
                "critic",
                format!("{summary_clone} (gaps: {})", gaps_clone.join(", ")),
            ));
        });
        self.emit(DeepResearchEvent::CritiqueRecorded { summary, gaps });
    }

    /// Record an arbitrary transcript step.
    pub fn push_transcript(&self, step: NodeStep) {
        let ev_step = step.clone();
        self.with_result(|r| r.transcript.push(step));
        self.emit(DeepResearchEvent::TranscriptStep { step: ev_step });
    }

    /// Set the coverage signals (typically called by the verifier).
    pub fn set_coverage(&self, coverage: CoverageSignals) {
        self.with_result(|r| r.coverage = coverage);
    }

    /// Mark citations verified / flagged after the verifier runs.
    pub fn mark_citation_status(&self, number: u32, status: CitationStatus) {
        self.with_result(|r| {
            if let Some(c) = r.citations.iter_mut().find(|c| c.number == number) {
                c.status = status;
            }
        });
    }

    /// Accumulate telemetry for a single role.
    pub fn accumulate_telemetry(
        &self,
        label: impl Into<String>,
        node: atomr_agents_deep_research_core::NodeTelemetry,
    ) {
        self.with_result(|r| r.telemetry.accumulate(label, node));
    }

    /// Replace the artifacts payload outright.
    pub fn set_artifacts(&self, artifacts: Artifacts) {
        self.with_result(|r| r.artifacts = artifacts);
    }

    /// Replace telemetry totals outright.
    pub fn set_telemetry(&self, t: Telemetry) {
        self.with_result(|r| r.telemetry = t);
    }

    /// Final state — emits the `Finalized` event.
    pub fn finalize(&self) {
        let (sections, citations, answered) = self.with_result(|r| {
            r.state = ResearchState::Done;
            let answered = r
                .plan
                .as_ref()
                .map(|p| {
                    p.sub_questions
                        .iter()
                        .filter(|s| s.status == SubQuestionStatus::Answered)
                        .count() as u32
                })
                .unwrap_or(0);
            (r.artifacts.drafts.len(), r.citations.len(), answered)
        });
        self.emit(DeepResearchEvent::Finalized {
            sections,
            citations,
            sub_questions_answered: answered,
        });
    }

    /// Mark the run as failed.
    pub fn fail(&self, reason: impl Into<String>) {
        let reason = reason.into();
        self.with_result(|r| {
            r.state = ResearchState::Failed;
            r.failure_reason = Some(reason.clone());
        });
        self.emit(DeepResearchEvent::Failed { reason });
    }

    /// Helper for the verifier: ensure the citation list is uniquely
    /// numbered and sorted by ascending number.
    pub fn renumber_citations(&self) {
        self.with_result(|r| {
            // Dedupe by URL, keep earliest.
            let mut seen: Vec<Url> = Vec::new();
            let mut deduped: Vec<Citation> = Vec::new();
            for c in r.citations.drain(..) {
                if !seen.iter().any(|u| u == &c.url) {
                    seen.push(c.url.clone());
                    deduped.push(c);
                }
            }
            for (i, c) in deduped.iter_mut().enumerate() {
                c.number = (i as u32) + 1;
            }
            r.citations = deduped;
        });
    }
}
