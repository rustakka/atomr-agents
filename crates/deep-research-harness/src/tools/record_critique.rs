//! `record_critique` — log one critic pass.

use async_trait::async_trait;
use atomr_agents_core::{AgentError, InvokeCtx, Result as CoreResult, ToolId, Value};
use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};
use serde::Deserialize;
use serde_json::json;

use crate::handle::ResearchHandle;

#[derive(Debug, Deserialize)]
struct Args {
    summary: String,
    #[serde(default)]
    gaps: Vec<String>,
}

/// Record one critic pass on the transcript.
pub struct RecordCritiqueTool {
    handle: ResearchHandle,
    descriptor: ToolDescriptor,
}

impl RecordCritiqueTool {
    pub fn new(handle: ResearchHandle) -> Self {
        let descriptor = ToolDescriptor {
            id: ToolId::from("deep_research.record_critique"),
            name: "record_critique".into(),
            description: "Record a critic pass (summary + list of gap tags) on the transcript. \
                 The harness uses the recorded gaps to decide whether to loop back to \
                 research or proceed to verify."
                .into(),
            schema: ToolSchema(json!({
                "type": "object",
                "required": ["summary"],
                "properties": {
                    "summary": { "type": "string", "minLength": 1 },
                    "gaps":    { "type": "array", "items": { "type": "string" } }
                }
            })),
        };
        Self { handle, descriptor }
    }
}

#[async_trait]
impl Tool for RecordCritiqueTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> CoreResult<Value> {
        let args: Args = serde_json::from_value(args)
            .map_err(|e| AgentError::Tool(format!("record_critique: invalid args: {e}")))?;
        self.handle.record_critique(args.summary, args.gaps);
        Ok(json!({}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::tests_support::{ctx, handle_for};
    use atomr_agents_deep_research_core::NodeKind;

    #[tokio::test]
    async fn records_critique() {
        let h = handle_for("q");
        let tool = RecordCritiqueTool::new(h.clone());
        tool.invoke(json!({ "summary": "1 gap", "gaps": ["unresolved:sq-1"] }), &ctx())
            .await
            .unwrap();
        assert!(h.snapshot().transcript.iter().any(|s| s.role == NodeKind::Critic));
    }

    #[tokio::test]
    async fn rejects_invalid_args() {
        let h = handle_for("q");
        let tool = RecordCritiqueTool::new(h);
        let err = tool.invoke(json!({}), &ctx()).await.unwrap_err();
        assert!(matches!(err, AgentError::Tool(_)));
    }
}
