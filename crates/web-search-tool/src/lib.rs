//! `WebSearchTool` — adapts any [`WebSearch`] provider into an
//! [`atomr_agents_tool::Tool`] so agents call it uniformly.

#![forbid(unsafe_code)]

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentError, InvokeCtx, Result as CoreResult, ToolId, Value};
use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};
use atomr_agents_web_search_core::{WebSearch, WebSearchRequest};
use serde_json::json;

/// Default tool id / name used when none is provided.
pub const DEFAULT_TOOL_NAME: &str = "web_search";

/// `Tool` impl backed by any `WebSearch` provider.
pub struct WebSearchTool {
    provider: Arc<dyn WebSearch>,
    descriptor: ToolDescriptor,
}

impl WebSearchTool {
    pub fn new(provider: Arc<dyn WebSearch>) -> Self {
        Self::with_name(provider, DEFAULT_TOOL_NAME)
    }

    pub fn with_name(provider: Arc<dyn WebSearch>, name: impl Into<String>) -> Self {
        let name = name.into();
        let descriptor = ToolDescriptor {
            id: ToolId::from(format!("web_search.{name}")),
            name,
            description: "Run a web search against the configured provider. Accepts a query \
                          plus optional max_results / allowed_domains / blocked_domains / \
                          recency_days. Returns { hits: [{ url, title, snippet, source, \
                          published, score, content }] }."
                .into(),
            schema: ToolSchema(json!({
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query":          { "type": "string", "minLength": 1 },
                    "max_results":    { "type": "integer", "minimum": 1, "maximum": 50 },
                    "allowed_domains":{ "type": "array", "items": { "type": "string" } },
                    "blocked_domains":{ "type": "array", "items": { "type": "string" } },
                    "recency_days":   { "type": "integer", "minimum": 1 },
                    "locale":         { "type": "string" },
                    "safe_search":    { "type": "string", "enum": ["off", "moderate", "strict"] }
                }
            })),
        };
        Self { provider, descriptor }
    }

    pub fn provider_name(&self) -> &str {
        self.provider.provider_name()
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> CoreResult<Value> {
        let request: WebSearchRequest = serde_json::from_value(args)
            .map_err(|e| AgentError::Tool(format!("web_search: invalid args: {e}")))?;
        let hits = self
            .provider
            .search(&request)
            .await
            .map_err(|e| AgentError::Tool(format!("web_search: {e}")))?;
        Ok(json!({ "hits": hits, "provider": self.provider.provider_name() }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_core::{CallCtx, IterationBudget, MoneyBudget, TimeBudget, TokenBudget};
    use atomr_agents_web_search_core::{MockWebSearch, WebSearchHit};
    use std::time::Duration;
    use url::Url;

    fn ctx() -> InvokeCtx {
        InvokeCtx {
            call: CallCtx {
                agent_id: None,
                tokens: TokenBudget::new(1000),
                time: TimeBudget::new(Duration::from_secs(5)),
                money: MoneyBudget::from_usd(1.0),
                iterations: IterationBudget::new(5),
                trace: vec![],
            },
            tool_call_id: "test-1".into(),
            raw_args: Value::Null,
        }
    }

    #[tokio::test]
    async fn tool_passes_through_to_provider() {
        let mock = MockWebSearch::new().with_fixture(
            "rust",
            vec![WebSearchHit::new(
                Url::parse("https://rust-lang.org/").unwrap(),
                "Rust",
                "Rust homepage",
            )],
        );
        let tool = WebSearchTool::new(Arc::new(mock));
        let out = tool
            .invoke(json!({ "query": "tell me about rust" }), &ctx())
            .await
            .unwrap();
        let hits = out.get("hits").and_then(|v| v.as_array()).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(out.get("provider").and_then(|v| v.as_str()), Some("mock"));
    }

    #[tokio::test]
    async fn tool_rejects_missing_query() {
        let tool = WebSearchTool::new(Arc::new(MockWebSearch::new()));
        let err = tool.invoke(json!({}), &ctx()).await.unwrap_err();
        match err {
            AgentError::Tool(m) => assert!(m.contains("invalid args")),
            other => panic!("expected Tool err, got {other:?}"),
        }
    }
}
