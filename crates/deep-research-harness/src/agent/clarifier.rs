//! LLM-driven [`Clarifier`] (Pattern B: one-shot JSON output).

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_deep_research_core::{NodeKind, NodeStep, ResearchRequest};
use serde::Deserialize;

use crate::agent::factory::InferenceClientFactory;
use crate::agent::parse::parse_json;
use crate::agent::prompts::CLARIFIER_PROMPT;
use crate::agent::strategies::{build_agent, default_budgets, make_inference, resolve_model};
use crate::error::{DeepResearchError, Result};
use crate::handle::ResearchHandle;
use crate::roles::{Clarifier, ClarifyOutcome};

const ROLE: &str = "clarifier";

/// Agent-backed [`Clarifier`].
///
/// Drives a single LLM turn with no tools; the model must respond with
/// a JSON object of the form `{"status":"ready"}` or `{"status":
/// "need_answers", "questions": [...]}`. Robust to fenced or
/// prose-wrapped responses (see [`crate::agent::parse::parse_json`]).
pub struct AgentBasedClarifier {
    factory: Arc<dyn InferenceClientFactory>,
    system_prompt: Option<String>,
    model_id: Option<String>,
    max_tool_iterations: u32,
}

impl AgentBasedClarifier {
    pub fn new(factory: Arc<dyn InferenceClientFactory>) -> Self {
        Self {
            factory,
            system_prompt: None,
            model_id: None,
            max_tool_iterations: 1,
        }
    }

    /// Override the default [`CLARIFIER_PROMPT`].
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Fall-back model id used when the request's `llm_overrides` are
    /// silent on the `clarifier` role.
    pub fn with_model_id(mut self, model_id: impl Into<String>) -> Self {
        self.model_id = Some(model_id.into());
        self
    }

    /// Cap the underlying agent's per-turn tool-loop length. Clarifier
    /// uses no tools so 1 is the sane default.
    pub fn with_max_tool_iterations(mut self, n: u32) -> Self {
        self.max_tool_iterations = n.max(1);
        self
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum RawOutcome {
    Ready,
    NeedAnswers { questions: Vec<String> },
}

#[async_trait]
impl Clarifier for AgentBasedClarifier {
    async fn clarify(&self, req: &ResearchRequest, handle: &ResearchHandle) -> Result<ClarifyOutcome> {
        // Always honor pre-supplied clarifications first.
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

        let model = resolve_model(req, ROLE, self.model_id.as_deref())?;
        let inference = make_inference(&self.factory, &model)?;
        let prompt = self
            .system_prompt
            .clone()
            .unwrap_or_else(|| CLARIFIER_PROMPT.to_string());
        let agent = build_agent(ROLE, model, prompt, inference, vec![], self.max_tool_iterations);

        let user = build_user_message(req);
        let turn = agent
            .run_turn(user, default_budgets(req))
            .await
            .map_err(|e| DeepResearchError::role(format!("clarifier turn failed: {e}")))?;

        let raw: RawOutcome = parse_json(&turn.text)?;
        let outcome = match raw {
            RawOutcome::Ready => ClarifyOutcome::Ready,
            RawOutcome::NeedAnswers { questions } => ClarifyOutcome::NeedAnswers { questions },
        };
        handle.push_transcript(NodeStep::new(
            NodeKind::Clarifier,
            "clarifier",
            match &outcome {
                ClarifyOutcome::Ready => "clarifier: ready".to_string(),
                ClarifyOutcome::NeedAnswers { questions } => {
                    format!("clarifier: needs {} answers", questions.len())
                }
            },
        ));
        Ok(outcome)
    }
}

fn build_user_message(req: &ResearchRequest) -> String {
    let mut buf = String::new();
    buf.push_str("Research query:\n");
    buf.push_str(&req.query);
    buf.push_str("\n\nHITL policy: ");
    buf.push_str(match req.human_in_the_loop {
        atomr_agents_deep_research_core::HitlPolicy::Off => "off",
        atomr_agents_deep_research_core::HitlPolicy::AskOnce => "ask_once",
        atomr_agents_deep_research_core::HitlPolicy::AskEveryRound => "ask_every_round",
        atomr_agents_deep_research_core::HitlPolicy::AutoClarify => "auto_clarify",
    });
    if let Some(bg) = &req.scope.background {
        buf.push_str("\n\nBackground:\n");
        buf.push_str(bg);
    }
    if !req.scope.allowed_domains.is_empty() {
        buf.push_str("\n\nAllowed domains: ");
        buf.push_str(&req.scope.allowed_domains.join(", "));
    }
    buf
}
