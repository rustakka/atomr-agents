//! `set_final_report` — set the final report body.

use async_trait::async_trait;
use atomr_agents_core::{AgentError, InvokeCtx, Result as CoreResult, ToolId, Value};
use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};
use serde::Deserialize;
use serde_json::json;

use crate::handle::ResearchHandle;

#[derive(Debug, Deserialize)]
struct Args {
    markdown: String,
}

/// Set the final report body on the handle.
pub struct SetFinalReportTool {
    handle: ResearchHandle,
    descriptor: ToolDescriptor,
}

impl SetFinalReportTool {
    pub fn new(handle: ResearchHandle) -> Self {
        let descriptor = ToolDescriptor {
            id: ToolId::from("deep_research.set_final_report"),
            name: "set_final_report".into(),
            description: "Set the final report body (markdown). The writer calls this once after \
                 emitting per-section drafts; subsequent calls overwrite."
                .into(),
            schema: ToolSchema(json!({
                "type": "object",
                "required": ["markdown"],
                "properties": {
                    "markdown": { "type": "string", "minLength": 1 }
                }
            })),
        };
        Self { handle, descriptor }
    }
}

#[async_trait]
impl Tool for SetFinalReportTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> CoreResult<Value> {
        let args: Args = serde_json::from_value(args)
            .map_err(|e| AgentError::Tool(format!("set_final_report: invalid args: {e}")))?;
        self.handle.set_final_report(args.markdown);
        Ok(json!({}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::tests_support::{ctx, handle_for};

    #[tokio::test]
    async fn sets_final_report() {
        let h = handle_for("q");
        let tool = SetFinalReportTool::new(h.clone());
        tool.invoke(json!({ "markdown": "# Title\n\nbody" }), &ctx())
            .await
            .unwrap();
        assert_eq!(h.snapshot().final_report.as_deref(), Some("# Title\n\nbody"));
    }

    #[tokio::test]
    async fn rejects_invalid_args() {
        let h = handle_for("q");
        let tool = SetFinalReportTool::new(h);
        let err = tool.invoke(json!({}), &ctx()).await.unwrap_err();
        assert!(matches!(err, AgentError::Tool(_)));
    }
}
