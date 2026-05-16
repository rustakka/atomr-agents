//! LLM-driven [`CitationVerifier`] (Pattern B: one-shot JSON output).

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_deep_research_core::{
    CitationStatus, CoverageSignals, NodeKind, NodeStep, ResearchRequest, SubQuestionStatus,
};
use serde::Deserialize;

use crate::agent::factory::InferenceClientFactory;
use crate::agent::parse::parse_json;
use crate::agent::prompts::VERIFIER_PROMPT;
use crate::agent::strategies::{build_agent, default_budgets, make_inference, resolve_model};
use crate::error::{DeepResearchError, Result};
use crate::handle::ResearchHandle;
use crate::roles::CitationVerifier;

const ROLE: &str = "verifier";

/// Agent-backed [`CitationVerifier`].
///
/// Asks the model to inspect every numbered citation and report which
/// look genuine and which look broken / off-topic / duplicated. The
/// harness applies the resulting verdicts via
/// [`ResearchHandle::mark_citation_status`] and computes coverage
/// signals from the current plan + drafts.
pub struct AgentBasedCitationVerifier {
    factory: Arc<dyn InferenceClientFactory>,
    system_prompt: Option<String>,
    model_id: Option<String>,
    max_tool_iterations: u32,
}

impl AgentBasedCitationVerifier {
    pub fn new(factory: Arc<dyn InferenceClientFactory>) -> Self {
        Self {
            factory,
            system_prompt: None,
            model_id: None,
            max_tool_iterations: 1,
        }
    }

    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    pub fn with_model_id(mut self, model_id: impl Into<String>) -> Self {
        self.model_id = Some(model_id.into());
        self
    }

    pub fn with_max_tool_iterations(mut self, n: u32) -> Self {
        self.max_tool_iterations = n.max(1);
        self
    }
}

#[derive(Debug, Deserialize)]
struct RawVerdicts {
    #[serde(default)]
    verdicts: Vec<RawVerdict>,
}

#[derive(Debug, Deserialize)]
struct RawVerdict {
    number: u32,
    status: String,
}

#[async_trait]
impl CitationVerifier for AgentBasedCitationVerifier {
    async fn verify(&self, handle: &ResearchHandle) -> Result<()> {
        let req: ResearchRequest = (*handle.request()).clone();
        // Renumber up front so the model sees a contiguous list.
        handle.renumber_citations();

        let snap_before = handle.snapshot();
        if snap_before.citations.is_empty() {
            // Nothing to verify; still compute coverage signals so the
            // pipeline stays consistent with the deterministic default.
            apply_coverage(handle);
            handle.push_transcript(NodeStep::new(
                NodeKind::Verifier,
                "verifier",
                "verifier: no citations to verify",
            ));
            return Ok(());
        }

        let model = resolve_model(&req, ROLE, self.model_id.as_deref())?;
        let inference = make_inference(&self.factory, &model)?;
        let prompt = self
            .system_prompt
            .clone()
            .unwrap_or_else(|| VERIFIER_PROMPT.to_string());
        let agent = build_agent(ROLE, model, prompt, inference, vec![], self.max_tool_iterations);

        let user = build_user_message(handle);
        let turn = agent
            .run_turn(user, default_budgets(&req))
            .await
            .map_err(|e| DeepResearchError::role(format!("verifier turn failed: {e}")))?;

        let raw: RawVerdicts = parse_json(&turn.text)?;
        for v in raw.verdicts {
            let status = match v.status.as_str() {
                "verified" => CitationStatus::Verified,
                "flagged" => CitationStatus::Flagged,
                _ => CitationStatus::Unverified,
            };
            handle.mark_citation_status(v.number, status);
        }

        apply_coverage(handle);
        handle.push_transcript(NodeStep::new(
            NodeKind::Verifier,
            "verifier",
            format!("verifier: applied {} verdicts", handle.snapshot().citations.len()),
        ));
        Ok(())
    }
}

fn apply_coverage(handle: &ResearchHandle) {
    let snap = handle.snapshot();
    let mut coverage = CoverageSignals::default();
    if let Some(plan) = &snap.plan {
        for sq in &plan.sub_questions {
            match sq.status {
                SubQuestionStatus::Answered => coverage.sub_questions_answered += 1,
                _ => coverage.sub_questions_unresolved += 1,
            }
        }
        let citation_re = regex::Regex::new(r"\[\d+\]").unwrap();
        for section in &snap.artifacts.drafts {
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
}

fn build_user_message(handle: &ResearchHandle) -> String {
    let snap = handle.snapshot();
    let mut buf = String::new();
    buf.push_str("Citations (review each and emit a verdict):\n");
    for c in &snap.citations {
        buf.push_str(&format!(
            "[{}] url={} title={} snippet={}\n",
            c.number, c.url, c.title, c.snippet
        ));
    }
    buf
}
