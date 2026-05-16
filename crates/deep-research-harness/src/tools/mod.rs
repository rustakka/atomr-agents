//! Domain tools for the deep-research harness.
//!
//! These tools wrap the most useful mutation methods on
//! [`ResearchHandle`] so an LLM-driven role (or any
//! [`atomr_agents_tool::Tool`]-aware caller) can drive a run. They are
//! always compiled — the `agent` feature only adds the
//! [`crate::agent`] module that consumes them — and mirror the
//! `ToolHandle` + `MeetingsToolSet` pattern from the meetings harness.
//!
//! Each tool:
//!
//! - Holds a cloned [`ResearchHandle`] and a static [`ToolDescriptor`].
//! - Validates its arguments against a tiny JSON schema.
//! - Calls the matching [`ResearchHandle`] method and returns either
//!   `{}` or `{ "id": "<...>" }` for tools that produce one.
//!
//! Build a [`ResearchToolSet`] once around a shared handle clone, then
//! plumb the resulting `Vec<Arc<dyn Tool>>` into a
//! [`atomr_agents_strategy::ToolStrategy`] (typically
//! `StaticToolStrategy::new(...)`) when configuring an
//! [`atomr_agents_agent::Agent`].

use std::sync::Arc;

use atomr_agents_tool::Tool;

use crate::handle::ResearchHandle;

mod append_citation;
mod append_draft_section;
mod append_sub_question;
mod record_clarification;
mod record_critique;
mod record_search_hit;
mod set_final_report;
mod set_plan;
mod set_sub_question_status;

#[cfg(test)]
mod tests_support;

pub use append_citation::AppendCitationTool;
pub use append_draft_section::AppendDraftSectionTool;
pub use append_sub_question::AppendSubQuestionTool;
pub use record_clarification::RecordClarificationTool;
pub use record_critique::RecordCritiqueTool;
pub use record_search_hit::RecordSearchHitTool;
pub use set_final_report::SetFinalReportTool;
pub use set_plan::SetPlanTool;
pub use set_sub_question_status::SetSubQuestionStatusTool;

/// Full bundle of domain tools that mutate a single
/// [`ResearchHandle`]. Construct one and pass `all()` (or a subset) to
/// a [`atomr_agents_strategy::ToolStrategy`] when wiring an agent.
pub struct ResearchToolSet {
    pub record_clarification: RecordClarificationTool,
    pub set_plan: SetPlanTool,
    pub append_sub_question: AppendSubQuestionTool,
    pub set_sub_question_status: SetSubQuestionStatusTool,
    pub record_search_hit: RecordSearchHitTool,
    pub append_citation: AppendCitationTool,
    pub append_draft_section: AppendDraftSectionTool,
    pub record_critique: RecordCritiqueTool,
    pub set_final_report: SetFinalReportTool,
}

impl ResearchToolSet {
    /// Build the full bundle around a shared [`ResearchHandle`] clone.
    pub fn new(handle: ResearchHandle) -> Self {
        Self {
            record_clarification: RecordClarificationTool::new(handle.clone()),
            set_plan: SetPlanTool::new(handle.clone()),
            append_sub_question: AppendSubQuestionTool::new(handle.clone()),
            set_sub_question_status: SetSubQuestionStatusTool::new(handle.clone()),
            record_search_hit: RecordSearchHitTool::new(handle.clone()),
            append_citation: AppendCitationTool::new(handle.clone()),
            append_draft_section: AppendDraftSectionTool::new(handle.clone()),
            record_critique: RecordCritiqueTool::new(handle.clone()),
            set_final_report: SetFinalReportTool::new(handle),
        }
    }

    /// Convenience: return every tool as `Arc<dyn Tool>` so the bundle
    /// can be plugged straight into a `StaticToolStrategy`.
    pub fn all(self) -> Vec<Arc<dyn Tool>> {
        vec![
            Arc::new(self.record_clarification),
            Arc::new(self.set_plan),
            Arc::new(self.append_sub_question),
            Arc::new(self.set_sub_question_status),
            Arc::new(self.record_search_hit),
            Arc::new(self.append_citation),
            Arc::new(self.append_draft_section),
            Arc::new(self.record_critique),
            Arc::new(self.set_final_report),
        ]
    }
}
