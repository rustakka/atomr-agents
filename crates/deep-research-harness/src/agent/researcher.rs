//! LLM-driven [`Researcher`] (Pattern A: tool-loop).
//!
//! Wires the agent with `web_search` (from `atomr-agents-web-search-tool`)
//! plus the citation / search-hit / status mutation tools so the model
//! can populate the running [`ResearchHandle`] without producing
//! free-form prose.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_deep_research_core::{NodeKind, NodeStep, ResearchRequest, SubQuestion};
use atomr_agents_tool::DynTool;
use atomr_agents_web_search_tool::WebSearchTool;

use crate::agent::factory::InferenceClientFactory;
use crate::agent::prompts::RESEARCHER_PROMPT;
use crate::agent::strategies::{build_agent, default_budgets, make_inference, resolve_model};
use crate::error::{DeepResearchError, Result};
use crate::handle::ResearchHandle;
use crate::roles::Researcher;
use crate::tools::{AppendCitationTool, RecordSearchHitTool, SetSubQuestionStatusTool};

const ROLE: &str = "researcher";

/// Agent-backed [`Researcher`].
///
/// Each call drives the model through `web_search` and the harness
/// mutation tools, then reads the resulting handle to update the
/// transcript. The model's free text is discarded — the tool calls
/// are the contract.
pub struct AgentBasedResearcher {
    factory: Arc<dyn InferenceClientFactory>,
    system_prompt: Option<String>,
    model_id: Option<String>,
    max_tool_iterations: u32,
}

impl AgentBasedResearcher {
    pub fn new(factory: Arc<dyn InferenceClientFactory>) -> Self {
        Self {
            factory,
            system_prompt: None,
            model_id: None,
            max_tool_iterations: 12,
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
impl Researcher for AgentBasedResearcher {
    async fn research(&self, sub: &SubQuestion, handle: &ResearchHandle) -> Result<()> {
        let req: ResearchRequest = (*handle.request()).clone();
        let model = resolve_model(&req, ROLE, self.model_id.as_deref())?;
        let inference = make_inference(&self.factory, &model)?;
        let prompt = self
            .system_prompt
            .clone()
            .unwrap_or_else(|| RESEARCHER_PROMPT.to_string());

        // Researcher toolbelt: web_search + record_search_hit +
        // append_citation + set_sub_question_status. Filtered by the
        // request's `tools_allowlist` when non-empty.
        let mut tools: Vec<DynTool> = vec![
            Arc::new(WebSearchTool::new(handle.search())),
            Arc::new(RecordSearchHitTool::new(handle.clone())),
            Arc::new(AppendCitationTool::new(handle.clone())),
            Arc::new(SetSubQuestionStatusTool::new(handle.clone())),
        ];
        if !req.tools_allowlist.is_empty() {
            tools.retain(|t| req.tools_allowlist.iter().any(|n| n == &t.descriptor().name));
        }

        let agent = build_agent(ROLE, model, prompt, inference, tools, self.max_tool_iterations);

        let user = build_user_message(&req, sub);
        let _turn = agent
            .run_turn(user, default_budgets(&req))
            .await
            .map_err(|e| DeepResearchError::role(format!("researcher turn failed: {e}")))?;

        let snap = handle.snapshot();
        let hits_for_sq = snap
            .artifacts
            .raw_search_hits
            .iter()
            .filter(|h| h.sub_question_id.as_deref() == Some(sub.id.as_str()))
            .count();
        handle.push_transcript(NodeStep {
            role: NodeKind::Researcher,
            label: format!("researcher:{}", sub.id),
            ts: chrono::Utc::now(),
            summary: format!(
                "researcher: {} hits recorded for sub-question `{}`",
                hits_for_sq, sub.text
            ),
            sub_question_id: Some(sub.id.clone()),
        });
        Ok(())
    }
}

fn build_user_message(req: &ResearchRequest, sub: &SubQuestion) -> String {
    let mut buf = String::new();
    buf.push_str("Sub-question to research:\n- id: ");
    buf.push_str(&sub.id);
    buf.push_str("\n- text: ");
    buf.push_str(&sub.text);
    if let Some(section) = &sub.section {
        buf.push_str("\n- section: ");
        buf.push_str(section);
    }
    buf.push_str(&format!(
        "\n\nMax search results per query: {}\n",
        req.breadth.max(1)
    ));
    if !req.scope.allowed_domains.is_empty() {
        buf.push_str(&format!(
            "Allowed domains: {}\n",
            req.scope.allowed_domains.join(", ")
        ));
    }
    if !req.scope.blocked_domains.is_empty() {
        buf.push_str(&format!(
            "Blocked domains: {}\n",
            req.scope.blocked_domains.join(", ")
        ));
    }
    buf
}
