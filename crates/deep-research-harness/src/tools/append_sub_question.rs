//! `append_sub_question` — push one sub-question onto the plan.

use async_trait::async_trait;
use atomr_agents_core::{AgentError, InvokeCtx, Result as CoreResult, ToolId, Value};
use atomr_agents_deep_research_core::SubQuestion;
use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};
use serde::Deserialize;
use serde_json::json;

use crate::handle::ResearchHandle;

#[derive(Debug, Deserialize)]
struct Args {
    sub_question: SubQuestion,
}

/// Append one [`SubQuestion`] to the running plan.
pub struct AppendSubQuestionTool {
    handle: ResearchHandle,
    descriptor: ToolDescriptor,
}

impl AppendSubQuestionTool {
    pub fn new(handle: ResearchHandle) -> Self {
        let descriptor = ToolDescriptor {
            id: ToolId::from("deep_research.append_sub_question"),
            name: "append_sub_question".into(),
            description: "Append one sub-question to the plan. Use after `set_plan` when a critic \
                 or re-plan step discovers a new gap. The plan is auto-created if missing."
                .into(),
            schema: ToolSchema(json!({
                "type": "object",
                "required": ["sub_question"],
                "properties": {
                    "sub_question": {
                        "type": "object",
                        "required": ["id", "text"],
                        "properties": {
                            "id":        { "type": "string", "minLength": 1 },
                            "text":      { "type": "string", "minLength": 1 },
                            "rationale": { "type": "string" },
                            "section":   { "type": "string" },
                            "status":    {
                                "type": "string",
                                "enum": ["pending", "in_progress", "answered", "unresolved"]
                            }
                        }
                    }
                }
            })),
        };
        Self { handle, descriptor }
    }
}

#[async_trait]
impl Tool for AppendSubQuestionTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> CoreResult<Value> {
        let args: Args = serde_json::from_value(args)
            .map_err(|e| AgentError::Tool(format!("append_sub_question: invalid args: {e}")))?;
        let id = self.handle.append_sub_question(args.sub_question);
        Ok(json!({ "id": id }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::tests_support::{ctx, handle_for};

    #[tokio::test]
    async fn appends_sub_question() {
        let h = handle_for("q");
        let tool = AppendSubQuestionTool::new(h.clone());
        let out = tool
            .invoke(
                json!({ "sub_question": { "id": "sq-x", "text": "tell me more" } }),
                &ctx(),
            )
            .await
            .unwrap();
        assert_eq!(out.get("id").and_then(|v| v.as_str()), Some("sq-x"));
        let plan = h.snapshot().plan.expect("plan auto-created");
        assert_eq!(plan.sub_questions.len(), 1);
        assert_eq!(plan.sub_questions[0].id, "sq-x");
    }

    #[tokio::test]
    async fn rejects_invalid_args() {
        let h = handle_for("q");
        let tool = AppendSubQuestionTool::new(h);
        let err = tool.invoke(json!({}), &ctx()).await.unwrap_err();
        assert!(matches!(err, AgentError::Tool(_)));
    }
}
