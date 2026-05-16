//! `set_plan` — replace the running plan.

use async_trait::async_trait;
use atomr_agents_core::{AgentError, InvokeCtx, Result as CoreResult, ToolId, Value};
use atomr_agents_deep_research_core::Plan;
use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};
use serde::Deserialize;
use serde_json::json;

use crate::handle::ResearchHandle;

#[derive(Debug, Deserialize)]
struct Args {
    plan: Plan,
}

/// Replace the in-flight [`Plan`] on the handle.
pub struct SetPlanTool {
    handle: ResearchHandle,
    descriptor: ToolDescriptor,
}

impl SetPlanTool {
    pub fn new(handle: ResearchHandle) -> Self {
        let descriptor = ToolDescriptor {
            id: ToolId::from("deep_research.set_plan"),
            name: "set_plan".into(),
            description: "Set (or replace) the research plan. Plan is `{ outline: [String], \
                 sub_questions: [{ id, text, rationale?, section?, status? }], \
                 rationale? }`. Status defaults to `pending`."
                .into(),
            schema: ToolSchema(json!({
                "type": "object",
                "required": ["plan"],
                "properties": {
                    "plan": {
                        "type": "object",
                        "properties": {
                            "outline":       { "type": "array", "items": { "type": "string" } },
                            "rationale":     { "type": "string" },
                            "sub_questions": {
                                "type": "array",
                                "items": {
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
                        }
                    }
                }
            })),
        };
        Self { handle, descriptor }
    }
}

#[async_trait]
impl Tool for SetPlanTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> CoreResult<Value> {
        let args: Args = serde_json::from_value(args)
            .map_err(|e| AgentError::Tool(format!("set_plan: invalid args: {e}")))?;
        self.handle.set_plan(args.plan);
        Ok(json!({}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::tests_support::{ctx, handle_for};

    #[tokio::test]
    async fn sets_plan_on_handle() {
        let h = handle_for("q");
        let tool = SetPlanTool::new(h.clone());
        tool.invoke(
            json!({
                "plan": {
                    "outline": ["Background", "Findings"],
                    "sub_questions": [
                        { "id": "sq-1", "text": "what?" },
                        { "id": "sq-2", "text": "how?" }
                    ],
                    "rationale": "split"
                }
            }),
            &ctx(),
        )
        .await
        .unwrap();
        let snap = h.snapshot();
        let plan = snap.plan.expect("plan set");
        assert_eq!(plan.outline.len(), 2);
        assert_eq!(plan.sub_questions.len(), 2);
        assert_eq!(plan.rationale.as_deref(), Some("split"));
    }

    #[tokio::test]
    async fn rejects_missing_plan() {
        let h = handle_for("q");
        let tool = SetPlanTool::new(h);
        let err = tool.invoke(json!({}), &ctx()).await.unwrap_err();
        assert!(matches!(err, AgentError::Tool(_)));
    }
}
