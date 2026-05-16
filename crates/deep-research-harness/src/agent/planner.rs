//! LLM-driven [`Planner`] (Pattern B: one-shot JSON output).

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_deep_research_core::{NodeKind, NodeStep, Plan, ResearchRequest};

use crate::agent::factory::InferenceClientFactory;
use crate::agent::parse::parse_json;
use crate::agent::prompts::PLANNER_PROMPT;
use crate::agent::strategies::{build_agent, default_budgets, make_inference, resolve_model};
use crate::error::{DeepResearchError, Result};
use crate::handle::ResearchHandle;
use crate::roles::Planner;

const ROLE: &str = "planner";

/// Agent-backed [`Planner`].
///
/// One LLM turn, no tools; the model must reply with a JSON object that
/// deserializes into [`Plan`].
pub struct AgentBasedPlanner {
    factory: Arc<dyn InferenceClientFactory>,
    system_prompt: Option<String>,
    model_id: Option<String>,
    max_tool_iterations: u32,
}

impl AgentBasedPlanner {
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

#[async_trait]
impl Planner for AgentBasedPlanner {
    async fn plan(&self, req: &ResearchRequest, handle: &ResearchHandle) -> Result<Plan> {
        let model = resolve_model(req, ROLE, self.model_id.as_deref())?;
        let inference = make_inference(&self.factory, &model)?;
        let prompt = self
            .system_prompt
            .clone()
            .unwrap_or_else(|| PLANNER_PROMPT.to_string());
        let agent = build_agent(ROLE, model, prompt, inference, vec![], self.max_tool_iterations);

        let user = build_user_message(req, handle);
        let turn = agent
            .run_turn(user, default_budgets(req))
            .await
            .map_err(|e| DeepResearchError::role(format!("planner turn failed: {e}")))?;
        let plan: Plan = parse_json(&turn.text)?;
        handle.push_transcript(NodeStep::new(
            NodeKind::Planner,
            "planner",
            format!("composed plan with {} sub-questions", plan.sub_questions.len()),
        ));
        Ok(plan)
    }
}

fn build_user_message(req: &ResearchRequest, _handle: &ResearchHandle) -> String {
    let mut buf = String::new();
    buf.push_str("Query:\n");
    buf.push_str(&req.query);
    buf.push_str(&format!(
        "\n\nTarget breadth (max sub-questions): {}\n",
        req.breadth
    ));
    if !req.clarifications.is_empty() {
        buf.push_str("\nClarifications:\n");
        for c in &req.clarifications {
            buf.push_str(&format!("- Q: {}\n  A: {}\n", c.question, c.answer));
        }
    }
    buf
}
