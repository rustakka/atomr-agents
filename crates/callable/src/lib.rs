//! `Callable` — the single composition abstraction for atomr-agents.

mod decorators;
mod pipeline;

pub use decorators::{
    with_config, with_fallbacks, with_retry, with_timeout, Branch, Lambda, WithConfig, WithFallbacks,
    WithRetry, WithTimeout,
};
pub use pipeline::{fan_out, Pipeline};

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{CallCtx, Result, Value};

/// Anything an agent / workflow / harness can call. Implemented by
/// every executable unit so they're interchangeable as workflow
/// steps, tool invocations, and team routing targets.
#[async_trait]
pub trait Callable: Send + Sync + 'static {
    async fn call(&self, input: Value, ctx: CallCtx) -> Result<Value>;

    /// Human-readable label for telemetry. Default falls back to the
    /// type name.
    fn label(&self) -> &str {
        std::any::type_name::<Self>()
    }
}

/// Type-erased handle. Crates that need to store heterogeneous
/// callables (registries, workflow steps, tool slots) use this.
pub type CallableHandle = Arc<dyn Callable>;

/// Convenience adapter: turn an async closure into a `Callable`.
///
/// ```ignore
/// let handle = FnCallable::new(|input, _ctx| async move { Ok(input) });
/// ```
pub struct FnCallable<F> {
    inner: F,
    label: &'static str,
}

impl<F> FnCallable<F> {
    pub fn new(f: F) -> Self {
        Self {
            inner: f,
            label: "fn",
        }
    }

    pub fn labeled(label: &'static str, f: F) -> Self {
        Self { inner: f, label }
    }
}

#[async_trait]
impl<F, Fut> Callable for FnCallable<F>
where
    F: Fn(Value, CallCtx) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<Value>> + Send + 'static,
{
    async fn call(&self, input: Value, ctx: CallCtx) -> Result<Value> {
        (self.inner)(input, ctx).await
    }

    fn label(&self) -> &str {
        self.label
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_core::{IterationBudget, MoneyBudget, TimeBudget, TokenBudget};
    use std::time::Duration;

    fn ctx() -> CallCtx {
        CallCtx {
            agent_id: None,
            tokens: TokenBudget::new(1000),
            time: TimeBudget::new(Duration::from_secs(10)),
            money: MoneyBudget::from_usd(1.0),
            iterations: IterationBudget::new(10),
            trace: vec![],
        }
    }

    #[tokio::test]
    async fn fn_callable_round_trips() {
        let c = FnCallable::new(|input: Value, _ctx| async move { Ok(input) });
        let v = serde_json::json!({"hello": "world"});
        let out = c.call(v.clone(), ctx()).await.unwrap();
        assert_eq!(out, v);
    }

    #[tokio::test]
    async fn handle_is_dyn_safe() {
        let h: CallableHandle =
            std::sync::Arc::new(FnCallable::labeled("echo", |input: Value, _ctx| async move {
                Ok(input)
            }));
        let out = h.call(serde_json::json!(42), ctx()).await.unwrap();
        assert_eq!(out, serde_json::json!(42));
        assert_eq!(h.label(), "echo");
    }
}
