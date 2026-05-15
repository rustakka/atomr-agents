//! Object-safe dispatch trait + the public type-erased handle.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::Callable;
use atomr_agents_core::{AgentError, CallCtx, HarnessId, Result as CoreResult, Value};
use atomr_agents_deep_research_core::ResearchRequest;

/// Object-safe trait every deep-research harness implements.
#[async_trait]
pub trait DeepResearchHarnessDispatch: Send + Sync + 'static {
    async fn dispatch(&self, request: ResearchRequest) -> CoreResult<Value>;
}

/// Public type-erased handle.
#[derive(Clone)]
pub struct DeepResearchHarnessRef {
    pub id: HarnessId,
    inner: Arc<dyn DeepResearchHarnessDispatch>,
}

impl DeepResearchHarnessRef {
    pub fn new(id: HarnessId, inner: Arc<dyn DeepResearchHarnessDispatch>) -> Self {
        Self { id, inner }
    }

    /// Run the harness and return the serialized [`ResearchResult`].
    pub async fn run(&self, request: ResearchRequest) -> CoreResult<Value> {
        self.inner.dispatch(request).await
    }
}

#[async_trait]
impl Callable for DeepResearchHarnessRef {
    async fn call(&self, input: Value, _ctx: CallCtx) -> CoreResult<Value> {
        let request: ResearchRequest = parse_request(input)?;
        self.run(request).await
    }

    fn label(&self) -> &str {
        self.id.as_str()
    }
}

/// Parse a JSON `Value` into a [`ResearchRequest`]. Accepts the full
/// request as a JSON object, or a bare string as shorthand for
/// `{"query": "..."}`.
pub fn parse_request(input: Value) -> CoreResult<ResearchRequest> {
    if let Some(s) = input.as_str() {
        return Ok(ResearchRequest::new(s));
    }
    serde_json::from_value(input)
        .map_err(|e| AgentError::Harness(format!("deep-research: invalid request: {e}")))
}
