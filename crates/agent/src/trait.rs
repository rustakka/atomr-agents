use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::Callable;
use atomr_agents_core::{AgentId, CallCtx, Result, Value};

use crate::inference::TurnResult;

/// Public, type-erased handle to an agent. Implements `Callable`,
/// so an agent can be passed wherever any executable unit is
/// expected (workflow steps, team routing targets).
pub struct AgentRef {
    pub id: AgentId,
    inner: Arc<dyn AgentDispatch>,
}

impl AgentRef {
    pub fn new(id: AgentId, inner: Arc<dyn AgentDispatch>) -> Self {
        Self { id, inner }
    }

    pub async fn turn(&self, user: String, ctx: CallCtx) -> Result<TurnResult> {
        self.inner.dispatch(user, ctx).await
    }
}

#[async_trait]
pub trait AgentDispatch: Send + Sync + 'static {
    async fn dispatch(&self, user: String, ctx: CallCtx) -> Result<TurnResult>;
}

#[async_trait]
impl Callable for AgentRef {
    async fn call(&self, input: Value, ctx: CallCtx) -> Result<Value> {
        // Treat the input as either a plain string or `{"user": "..."}`.
        let user = match input {
            Value::String(s) => s,
            Value::Object(ref m) => m
                .get("user")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            _ => input.to_string(),
        };
        let r = self.turn(user, ctx).await?;
        Ok(serde_json::json!({
            "text": r.text,
            "input_tokens": r.usage.input_tokens,
            "output_tokens": r.usage.output_tokens,
        }))
    }

    fn label(&self) -> &str {
        self.id.as_str()
    }
}
