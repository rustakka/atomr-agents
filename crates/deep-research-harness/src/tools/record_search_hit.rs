//! `record_search_hit` — push one raw search hit into artifacts.

use async_trait::async_trait;
use atomr_agents_core::{AgentError, InvokeCtx, Result as CoreResult, ToolId, Value};
use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};
use atomr_agents_web_search_core::WebSearchHit;
use serde::Deserialize;
use serde_json::json;

use crate::handle::ResearchHandle;

#[derive(Debug, Deserialize)]
struct Args {
    provider: String,
    hit: WebSearchHit,
    #[serde(default)]
    sub_question_id: Option<String>,
}

/// Record one raw search hit on the handle. Intended to be paired with
/// `web_search`: the agent calls `web_search`, then loops the returned
/// `hits` array through this tool.
pub struct RecordSearchHitTool {
    handle: ResearchHandle,
    descriptor: ToolDescriptor,
}

impl RecordSearchHitTool {
    pub fn new(handle: ResearchHandle) -> Self {
        let descriptor = ToolDescriptor {
            id: ToolId::from("deep_research.record_search_hit"),
            name: "record_search_hit".into(),
            description: "Record one raw search hit in the artifacts ledger. `hit` is one element \
                 of the `hits` array returned by `web_search`. Optionally associate it \
                 with the sub-question id that motivated the search."
                .into(),
            schema: ToolSchema(json!({
                "type": "object",
                "required": ["provider", "hit"],
                "properties": {
                    "provider":        { "type": "string", "minLength": 1 },
                    "sub_question_id": { "type": "string" },
                    "hit": {
                        "type": "object",
                        "required": ["url", "title", "snippet", "source"],
                        "properties": {
                            "url":       { "type": "string", "format": "uri" },
                            "title":     { "type": "string" },
                            "snippet":   { "type": "string" },
                            "source":    { "type": "string" },
                            "published": { "type": "string", "format": "date-time" },
                            "score":     { "type": "number" },
                            "content":   { "type": "string" }
                        }
                    }
                }
            })),
        };
        Self { handle, descriptor }
    }
}

#[async_trait]
impl Tool for RecordSearchHitTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> CoreResult<Value> {
        let args: Args = serde_json::from_value(args)
            .map_err(|e| AgentError::Tool(format!("record_search_hit: invalid args: {e}")))?;
        self.handle
            .record_search_hit(args.provider, &args.hit, args.sub_question_id);
        Ok(json!({}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::tests_support::{ctx, handle_for};

    #[tokio::test]
    async fn records_search_hit() {
        let h = handle_for("q");
        let tool = RecordSearchHitTool::new(h.clone());
        tool.invoke(
            json!({
                "provider": "mock",
                "hit": {
                    "url": "https://a.test/",
                    "title": "A",
                    "snippet": "snippet",
                    "source": "a.test"
                },
                "sub_question_id": "sq-1"
            }),
            &ctx(),
        )
        .await
        .unwrap();
        let raw = &h.snapshot().artifacts.raw_search_hits;
        assert_eq!(raw.len(), 1);
        assert_eq!(raw[0].title, "A");
        assert_eq!(raw[0].sub_question_id.as_deref(), Some("sq-1"));
    }

    #[tokio::test]
    async fn rejects_invalid_args() {
        let h = handle_for("q");
        let tool = RecordSearchHitTool::new(h);
        let err = tool.invoke(json!({}), &ctx()).await.unwrap_err();
        assert!(matches!(err, AgentError::Tool(_)));
    }
}
