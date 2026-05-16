//! LLM-driven [`Critic`] (Pattern B: one-shot JSON output).

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_deep_research_core::{NodeKind, NodeStep, ResearchRequest};
use serde::Deserialize;

use crate::agent::factory::InferenceClientFactory;
use crate::agent::parse::parse_json;
use crate::agent::prompts::CRITIC_PROMPT;
use crate::agent::strategies::{build_agent, default_budgets, make_inference, resolve_model};
use crate::error::{DeepResearchError, Result};
use crate::handle::ResearchHandle;
use crate::roles::{Critic, CritiqueOutcome};

const ROLE: &str = "critic";

/// Agent-backed [`Critic`].
pub struct AgentBasedCritic {
    factory: Arc<dyn InferenceClientFactory>,
    system_prompt: Option<String>,
    model_id: Option<String>,
    max_tool_iterations: u32,
}

impl AgentBasedCritic {
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
struct RawCritique {
    summary: String,
    #[serde(default)]
    gaps: Vec<String>,
    done: bool,
}

#[async_trait]
impl Critic for AgentBasedCritic {
    async fn critique(&self, handle: &ResearchHandle) -> Result<CritiqueOutcome> {
        let req: ResearchRequest = (*handle.request()).clone();
        let model = resolve_model(&req, ROLE, self.model_id.as_deref())?;
        let inference = make_inference(&self.factory, &model)?;
        let prompt = self
            .system_prompt
            .clone()
            .unwrap_or_else(|| CRITIC_PROMPT.to_string());
        let agent = build_agent(ROLE, model, prompt, inference, vec![], self.max_tool_iterations);

        let user = build_user_message(handle);
        let turn = agent
            .run_turn(user, default_budgets(&req))
            .await
            .map_err(|e| DeepResearchError::role(format!("critic turn failed: {e}")))?;

        let raw: RawCritique = parse_json(&turn.text)?;
        let outcome = CritiqueOutcome {
            summary: raw.summary,
            gaps: raw.gaps,
            done: raw.done,
        };
        handle.record_critique(outcome.summary.clone(), outcome.gaps.clone());
        handle.push_transcript(NodeStep::new(
            NodeKind::Critic,
            "critic",
            format!("critic: {} ({} gaps)", outcome.summary, outcome.gaps.len()),
        ));
        Ok(outcome)
    }
}

fn build_user_message(handle: &ResearchHandle) -> String {
    let snap = handle.snapshot();
    let mut buf = String::new();
    buf.push_str("Query: ");
    buf.push_str(&snap.query);
    if let Some(plan) = &snap.plan {
        buf.push_str("\n\nOutline:\n");
        for h in &plan.outline {
            buf.push_str(&format!("- {h}\n"));
        }
        buf.push_str("\nSub-questions:\n");
        for sq in &plan.sub_questions {
            buf.push_str(&format!("- [{}] {} (status: {:?})\n", sq.id, sq.text, sq.status));
        }
    }
    if !snap.artifacts.drafts.is_empty() {
        buf.push_str("\nDraft sections:\n");
        for section in &snap.artifacts.drafts {
            buf.push_str(&format!("## {}\n{}\n\n", section.heading, section.body));
        }
    }
    if !snap.citations.is_empty() {
        buf.push_str("\nCitations:\n");
        for c in &snap.citations {
            buf.push_str(&format!("[{}] {} — {}\n", c.number, c.title, c.url));
        }
    }
    buf
}
