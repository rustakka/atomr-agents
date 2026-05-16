//! `append_citation` — push one numbered citation onto the ledger.

use async_trait::async_trait;
use atomr_agents_core::{AgentError, InvokeCtx, Result as CoreResult, ToolId, Value};
use atomr_agents_deep_research_core::Citation;
use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};
use serde::Deserialize;
use serde_json::json;

use crate::handle::ResearchHandle;

#[derive(Debug, Deserialize)]
struct Args {
    citation: Citation,
}

/// Append a [`Citation`]; auto-renumbers when `number == 0`.
pub struct AppendCitationTool {
    handle: ResearchHandle,
    descriptor: ToolDescriptor,
}

impl AppendCitationTool {
    pub fn new(handle: ResearchHandle) -> Self {
        let descriptor = ToolDescriptor {
            id: ToolId::from("deep_research.append_citation"),
            name: "append_citation".into(),
            description: "Append a citation to the running ledger. Set `number` to 0 to auto-number \
                 sequentially (recommended). `supports` lists the sub-question ids the \
                 citation backs; the writer renders `[N]` markers from this."
                .into(),
            schema: ToolSchema(json!({
                "type": "object",
                "required": ["citation"],
                "properties": {
                    "citation": {
                        "type": "object",
                        "required": ["number", "url", "title", "snippet"],
                        "properties": {
                            "number":   { "type": "integer", "minimum": 0 },
                            "url":      { "type": "string", "format": "uri" },
                            "title":    { "type": "string" },
                            "snippet":  { "type": "string" },
                            "source":   { "type": "string" },
                            "supports": { "type": "array", "items": { "type": "string" } },
                            "status":   {
                                "type": "string",
                                "enum": ["unverified", "verified", "flagged"]
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
impl Tool for AppendCitationTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> CoreResult<Value> {
        let args: Args = serde_json::from_value(args)
            .map_err(|e| AgentError::Tool(format!("append_citation: invalid args: {e}")))?;
        let number = self.handle.append_citation(args.citation);
        Ok(json!({ "number": number }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::tests_support::{ctx, handle_for};

    #[tokio::test]
    async fn appends_citation_and_returns_number() {
        let h = handle_for("q");
        let tool = AppendCitationTool::new(h.clone());
        let out = tool
            .invoke(
                json!({
                    "citation": {
                        "number": 0,
                        "url": "https://a.test/",
                        "title": "A",
                        "snippet": "s",
                        "source": "a.test",
                        "supports": ["sq-1"]
                    }
                }),
                &ctx(),
            )
            .await
            .unwrap();
        assert_eq!(out.get("number").and_then(|v| v.as_u64()), Some(1));
        let cites = h.snapshot().citations;
        assert_eq!(cites.len(), 1);
        assert_eq!(cites[0].number, 1);
        assert_eq!(cites[0].supports, vec!["sq-1".to_string()]);
    }

    #[tokio::test]
    async fn rejects_invalid_args() {
        let h = handle_for("q");
        let tool = AppendCitationTool::new(h);
        let err = tool.invoke(json!({}), &ctx()).await.unwrap_err();
        assert!(matches!(err, AgentError::Tool(_)));
    }
}
