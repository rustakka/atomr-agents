//! `append_draft_section` — push one section into the running draft.

use async_trait::async_trait;
use atomr_agents_core::{AgentError, InvokeCtx, Result as CoreResult, ToolId, Value};
use atomr_agents_deep_research_core::DraftSection;
use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};
use serde::Deserialize;
use serde_json::json;

use crate::handle::ResearchHandle;

#[derive(Debug, Deserialize)]
struct Args {
    section: DraftSection,
}

/// Append one [`DraftSection`] to `artifacts.drafts`.
pub struct AppendDraftSectionTool {
    handle: ResearchHandle,
    descriptor: ToolDescriptor,
}

impl AppendDraftSectionTool {
    pub fn new(handle: ResearchHandle) -> Self {
        let descriptor = ToolDescriptor {
            id: ToolId::from("deep_research.append_draft_section"),
            name: "append_draft_section".into(),
            description: "Append one draft section (heading + markdown body + answered \
                 sub-question ids) to the running draft. Use one call per outline \
                 heading; the writer assembles them into the final report."
                .into(),
            schema: ToolSchema(json!({
                "type": "object",
                "required": ["section"],
                "properties": {
                    "section": {
                        "type": "object",
                        "required": ["heading", "body"],
                        "properties": {
                            "heading":                { "type": "string", "minLength": 1 },
                            "body":                   { "type": "string" },
                            "answers_sub_questions":  {
                                "type": "array",
                                "items": { "type": "string" }
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
impl Tool for AppendDraftSectionTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> CoreResult<Value> {
        let args: Args = serde_json::from_value(args)
            .map_err(|e| AgentError::Tool(format!("append_draft_section: invalid args: {e}")))?;
        self.handle.append_draft_section(args.section);
        Ok(json!({}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::tests_support::{ctx, handle_for};

    #[tokio::test]
    async fn appends_draft_section() {
        let h = handle_for("q");
        let tool = AppendDraftSectionTool::new(h.clone());
        tool.invoke(
            json!({
                "section": {
                    "heading": "Background",
                    "body": "Some prose [1].",
                    "answers_sub_questions": ["sq-1"]
                }
            }),
            &ctx(),
        )
        .await
        .unwrap();
        let drafts = h.snapshot().artifacts.drafts;
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].heading, "Background");
        assert_eq!(drafts[0].answers_sub_questions, vec!["sq-1".to_string()]);
    }

    #[tokio::test]
    async fn rejects_invalid_args() {
        let h = handle_for("q");
        let tool = AppendDraftSectionTool::new(h);
        let err = tool.invoke(json!({}), &ctx()).await.unwrap_err();
        assert!(matches!(err, AgentError::Tool(_)));
    }
}
