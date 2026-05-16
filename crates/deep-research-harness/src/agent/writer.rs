//! LLM-driven [`Writer`] (Pattern A: tool-loop).
//!
//! Wires the agent with [`AppendDraftSectionTool`] +
//! [`SetFinalReportTool`] so the model emits the report by calling
//! tools — no free-form prose is required.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_deep_research_core::{NodeKind, NodeStep, Plan, ResearchRequest};
use atomr_agents_tool::DynTool;

use crate::agent::factory::InferenceClientFactory;
use crate::agent::prompts::WRITER_PROMPT;
use crate::agent::strategies::{build_agent, default_budgets, make_inference, resolve_model};
use crate::error::{DeepResearchError, Result};
use crate::handle::ResearchHandle;
use crate::roles::Writer;
use crate::tools::{AppendDraftSectionTool, SetFinalReportTool};

const ROLE: &str = "writer";

/// Agent-backed [`Writer`].
pub struct AgentBasedWriter {
    factory: Arc<dyn InferenceClientFactory>,
    system_prompt: Option<String>,
    model_id: Option<String>,
    max_tool_iterations: u32,
}

impl AgentBasedWriter {
    pub fn new(factory: Arc<dyn InferenceClientFactory>) -> Self {
        Self {
            factory,
            system_prompt: None,
            model_id: None,
            max_tool_iterations: 8,
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
impl Writer for AgentBasedWriter {
    async fn write(&self, plan: &Plan, handle: &ResearchHandle) -> Result<()> {
        let req: ResearchRequest = (*handle.request()).clone();
        let model = resolve_model(&req, ROLE, self.model_id.as_deref())?;
        let inference = make_inference(&self.factory, &model)?;
        let prompt = self
            .system_prompt
            .clone()
            .unwrap_or_else(|| WRITER_PROMPT.to_string());

        let mut tools: Vec<DynTool> = vec![
            Arc::new(AppendDraftSectionTool::new(handle.clone())),
            Arc::new(SetFinalReportTool::new(handle.clone())),
        ];
        if !req.tools_allowlist.is_empty() {
            tools.retain(|t| req.tools_allowlist.iter().any(|n| n == &t.descriptor().name));
        }

        let agent = build_agent(ROLE, model, prompt, inference, tools, self.max_tool_iterations);

        let user = build_user_message(plan, handle);
        let _turn = agent
            .run_turn(user, default_budgets(&req))
            .await
            .map_err(|e| DeepResearchError::role(format!("writer turn failed: {e}")))?;

        let snap = handle.snapshot();
        handle.push_transcript(NodeStep::new(
            NodeKind::Writer,
            "writer",
            format!(
                "writer: drafted {} sections; final_report={}",
                snap.artifacts.drafts.len(),
                snap.final_report.is_some()
            ),
        ));
        Ok(())
    }
}

fn build_user_message(plan: &Plan, handle: &ResearchHandle) -> String {
    let snap = handle.snapshot();
    let mut buf = String::new();
    buf.push_str("Query: ");
    buf.push_str(&snap.query);
    buf.push_str("\n\nOutline:\n");
    for h in &plan.outline {
        buf.push_str(&format!("- {h}\n"));
    }
    buf.push_str("\nSub-questions:\n");
    for sq in &plan.sub_questions {
        buf.push_str(&format!(
            "- [{}] {} (section: {})\n",
            sq.id,
            sq.text,
            sq.section.as_deref().unwrap_or("-")
        ));
    }
    if !snap.citations.is_empty() {
        buf.push_str("\nCitations:\n");
        for c in &snap.citations {
            buf.push_str(&format!(
                "[{}] {} — {}\n    snippet: {}\n    supports: {}\n",
                c.number,
                c.title,
                c.url,
                c.snippet,
                c.supports.join(",")
            ));
        }
    }
    buf
}
