use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::Callable;
use atomr_agents_core::{CallCtx, InvokeCtx, Result, Value};

use crate::descriptor::ToolDescriptor;

/// A tool the agent may call. Tools are also `Callable`, so they
/// can stand in as workflow steps.
#[async_trait]
pub trait Tool: Send + Sync + 'static {
    fn descriptor(&self) -> &ToolDescriptor;
    async fn invoke(&self, args: Value, ctx: &InvokeCtx) -> Result<Value>;
}

/// `Arc<dyn Tool>` — what the registry stores.
pub type DynTool = Arc<dyn Tool>;

/// Adapter so any `Tool` exposes itself as a `Callable`. The adapter
/// fabricates an `InvokeCtx` by promoting the `CallCtx` and using a
/// synthetic tool-call-id.
pub struct ToolCallable<T: Tool> {
    inner: Arc<T>,
}

impl<T: Tool> ToolCallable<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner: Arc::new(inner),
        }
    }

    #[allow(dead_code)]
    pub fn from_arc(inner: Arc<T>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl<T: Tool> Callable for ToolCallable<T> {
    async fn call(&self, input: Value, ctx: CallCtx) -> Result<Value> {
        let invoke_ctx = InvokeCtx {
            call: ctx,
            tool_call_id: format!("synth-{}", uuid_like()),
            raw_args: input.clone(),
        };
        self.inner.invoke(input, &invoke_ctx).await
    }

    fn label(&self) -> &str {
        &self.inner.descriptor().name
    }
}

fn uuid_like() -> String {
    // Tiny non-crypto id; uuid is in atomr-agents-core but not
    // re-exported here to keep the dep graph small.
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!("{:016x}", N.fetch_add(1, Ordering::Relaxed))
}
