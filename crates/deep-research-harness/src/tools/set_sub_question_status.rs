//! `set_sub_question_status` — patch one sub-question's status.

use async_trait::async_trait;
use atomr_agents_core::{AgentError, InvokeCtx, Result as CoreResult, ToolId, Value};
use atomr_agents_deep_research_core::SubQuestionStatus;
use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};
use serde::Deserialize;
use serde_json::json;

use crate::handle::ResearchHandle;

#[derive(Debug, Deserialize)]
struct Args {
    sub_question_id: String,
    status: SubQuestionStatus,
}

/// Update a sub-question's `status` (e.g. `pending → answered`).
pub struct SetSubQuestionStatusTool {
    handle: ResearchHandle,
    descriptor: ToolDescriptor,
}

impl SetSubQuestionStatusTool {
    pub fn new(handle: ResearchHandle) -> Self {
        let descriptor = ToolDescriptor {
            id: ToolId::from("deep_research.set_sub_question_status"),
            name: "set_sub_question_status".into(),
            description: "Mark a sub-question with a new status (pending | in_progress | answered \
                 | unresolved). The researcher uses this to signal completion. Returns \
                 an error if the id is unknown."
                .into(),
            schema: ToolSchema(json!({
                "type": "object",
                "required": ["sub_question_id", "status"],
                "properties": {
                    "sub_question_id": { "type": "string", "minLength": 1 },
                    "status": {
                        "type": "string",
                        "enum": ["pending", "in_progress", "answered", "unresolved"]
                    }
                }
            })),
        };
        Self { handle, descriptor }
    }
}

#[async_trait]
impl Tool for SetSubQuestionStatusTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> CoreResult<Value> {
        let args: Args = serde_json::from_value(args)
            .map_err(|e| AgentError::Tool(format!("set_sub_question_status: invalid args: {e}")))?;
        self.handle
            .set_sub_question_status(&args.sub_question_id, args.status)
            .map_err(|e| AgentError::Tool(e.to_string()))?;
        Ok(json!({}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::tests_support::{ctx, handle_for};
    use atomr_agents_deep_research_core::SubQuestion;

    #[tokio::test]
    async fn sets_status() {
        let h = handle_for("q");
        h.append_sub_question(SubQuestion::new("sq-1", "x"));
        let tool = SetSubQuestionStatusTool::new(h.clone());
        tool.invoke(json!({ "sub_question_id": "sq-1", "status": "answered" }), &ctx())
            .await
            .unwrap();
        let plan = h.snapshot().plan.unwrap();
        assert_eq!(plan.sub_questions[0].status, SubQuestionStatus::Answered);
    }

    #[tokio::test]
    async fn unknown_id_errors() {
        let h = handle_for("q");
        let tool = SetSubQuestionStatusTool::new(h);
        let err = tool
            .invoke(
                json!({ "sub_question_id": "missing", "status": "answered" }),
                &ctx(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AgentError::Tool(_)));
    }
}
