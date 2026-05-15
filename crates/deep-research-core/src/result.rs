//! Uniform research result — the output every strategy produces.

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::artifacts::Artifacts;
use crate::citation::Citation;
use crate::coverage::CoverageSignals;
use crate::plan::Plan;
use crate::request::Markdown;
use crate::telemetry::Telemetry;
use crate::transcript::NodeStep;

/// Lifecycle of a research run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchState {
    /// Stored, not yet started.
    #[default]
    Pending,
    /// Clarifier composing or awaiting answers.
    Clarifying,
    /// Roles are actively producing draft content.
    Running,
    /// Verifier is running.
    Verifying,
    /// Run completed successfully.
    Done,
    /// Run aborted with an error.
    Failed,
}

/// Uniform output across every research strategy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchResult {
    /// Stable identifier (uuid-shaped).
    pub id: String,
    /// Original query for convenience.
    pub query: String,
    /// Name of the strategy that produced this result, e.g.
    /// `"clarify-plan-search-verify"`.
    pub strategy: String,
    pub state: ResearchState,

    /// Final report body. `Some` when `state == Done`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_report: Option<Markdown>,

    /// Numbered, deduplicated citations referenced from the report.
    #[serde(default)]
    pub citations: Vec<Citation>,

    /// Plan composed by the planner role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<Plan>,

    /// Per-node audit trail.
    #[serde(default)]
    pub transcript: Vec<NodeStep>,

    /// Coverage signals.
    #[serde(default)]
    pub coverage: CoverageSignals,

    /// Telemetry totals + per-node breakdown.
    #[serde(default)]
    pub telemetry: Telemetry,

    /// Intermediate drafts, raw hits, scratchpad.
    #[serde(default)]
    pub artifacts: Artifacts,

    /// Model id used for the run (if LLM-driven).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,

    /// Optional failure reason, populated when `state == Failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,

    /// Millis-since-epoch at creation.
    pub created_at_ms: i64,
    /// Millis-since-epoch at last touch.
    pub updated_at_ms: i64,
}

impl ResearchResult {
    /// Build a fresh `Pending` result tagged with the given strategy.
    pub fn new(query: impl Into<String>, strategy: impl Into<String>) -> Self {
        let now = Utc::now().timestamp_millis();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            query: query.into(),
            strategy: strategy.into(),
            state: ResearchState::Pending,
            final_report: None,
            citations: Vec::new(),
            plan: None,
            transcript: Vec::new(),
            coverage: CoverageSignals::default(),
            telemetry: Telemetry::default(),
            artifacts: Artifacts::default(),
            model_id: None,
            failure_reason: None,
            created_at_ms: now,
            updated_at_ms: now,
        }
    }

    /// Bump `updated_at_ms` to "now".
    pub fn touch(&mut self) {
        self.updated_at_ms = Utc::now().timestamp_millis();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::citation::Citation;
    use crate::plan::{Plan, SubQuestion};
    use crate::request::ResearchRequest;
    use crate::transcript::{NodeKind, NodeStep};
    use url::Url;

    #[test]
    fn fresh_result_starts_pending() {
        let r = ResearchResult::new("q", "s");
        assert_eq!(r.state, ResearchState::Pending);
        assert!(r.id.len() > 8);
        assert!(r.citations.is_empty());
    }

    #[test]
    fn result_round_trips_with_all_fields() {
        let mut r = ResearchResult::new("query", "clarify-plan-search-verify");
        r.state = ResearchState::Done;
        r.final_report = Some("# Title\n\nbody [1]".into());
        r.citations.push(Citation::new(
            1,
            Url::parse("https://example.com/").unwrap(),
            "Example",
            "snippet",
        ));
        let mut plan = Plan::new();
        plan.outline.push("Intro".into());
        plan.sub_questions.push(SubQuestion::new("sq-1", "what?"));
        r.plan = Some(plan);
        r.transcript
            .push(NodeStep::new(NodeKind::Writer, "writer", "wrote 1 section"));
        let j = serde_json::to_string(&r).unwrap();
        let back: ResearchResult = serde_json::from_str(&j).unwrap();
        assert_eq!(back.state, ResearchState::Done);
        assert_eq!(back.citations.len(), 1);
        assert_eq!(back.plan.unwrap().sub_questions.len(), 1);
        assert_eq!(back.transcript.len(), 1);
    }

    #[test]
    fn request_and_result_compose() {
        // Just check the contract types are usable together.
        let req = ResearchRequest::new("q").with_depth(3);
        let result = ResearchResult::new(req.query.clone(), "any");
        assert_eq!(result.query, "q");
    }
}
