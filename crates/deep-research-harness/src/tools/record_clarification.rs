//! `record_clarification` — append a clarifier Q/A turn.

use async_trait::async_trait;
use atomr_agents_core::{AgentError, InvokeCtx, Result as CoreResult, ToolId, Value};
use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};
use serde::Deserialize;
use serde_json::json;

use crate::handle::ResearchHandle;

#[derive(Debug, Deserialize)]
struct Args {
    question: String,
    answer: String,
}

/// Records one clarification Q/A on the transcript.
pub struct RecordClarificationTool {
    handle: ResearchHandle,
    descriptor: ToolDescriptor,
}

impl RecordClarificationTool {
    pub fn new(handle: ResearchHandle) -> Self {
        let descriptor = ToolDescriptor {
            id: ToolId::from("deep_research.record_clarification"),
            name: "record_clarification".into(),
            description: "Record a clarification Q/A turn on the running research transcript. Use \
                 when an early-phase agent has asked a clarifying question and now has \
                 the user's (or auto-derived) answer."
                .into(),
            schema: ToolSchema(json!({
                "type": "object",
                "required": ["question", "answer"],
                "properties": {
                    "question": { "type": "string", "minLength": 1 },
                    "answer":   { "type": "string", "minLength": 1 }
                }
            })),
        };
        Self { handle, descriptor }
    }
}

#[async_trait]
impl Tool for RecordClarificationTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> CoreResult<Value> {
        let args: Args = serde_json::from_value(args)
            .map_err(|e| AgentError::Tool(format!("record_clarification: invalid args: {e}")))?;
        self.handle.record_clarification(args.question, args.answer);
        Ok(json!({}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::tests_support::{ctx, handle_for};
    use atomr_agents_deep_research_core::NodeKind;

    #[tokio::test]
    async fn records_clarification_on_transcript() {
        let h = handle_for("q");
        let tool = RecordClarificationTool::new(h.clone());
        tool.invoke(json!({ "question": "Scope?", "answer": "rust" }), &ctx())
            .await
            .unwrap();
        let snap = h.snapshot();
        assert!(snap.transcript.iter().any(|s| s.role == NodeKind::Clarifier));
    }

    #[tokio::test]
    async fn rejects_missing_args() {
        let h = handle_for("q");
        let tool = RecordClarificationTool::new(h);
        let err = tool.invoke(json!({}), &ctx()).await.unwrap_err();
        match err {
            AgentError::Tool(m) => assert!(m.contains("invalid args")),
            other => panic!("expected Tool err, got {other:?}"),
        }
    }
}
