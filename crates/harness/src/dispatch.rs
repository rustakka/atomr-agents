//! Object-safe dispatch trait + boxed handle for `Harness`.
//!
//! Mirrors the `AgentDispatch` / `AgentRef` pattern in
//! `atomr-agents-agent::r#trait`. The typed `Harness<L, T>` keeps the
//! hot path monomorphized; `HarnessRef` exposes a uniform handle that
//! Python bindings (and any other consumer that can't name the strategy
//! generics) can hold and invoke.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::Callable;
use atomr_agents_core::{CallCtx, HarnessId, Result, Value};

/// Object-safe trait that every harness implements. Implementors are
/// the typed `Harness<L, T>` and the fully-erased `BoxedHarness`.
#[async_trait]
pub trait HarnessDispatch: Send + Sync + 'static {
    async fn dispatch(&self) -> Result<Value>;
}

/// Public, type-erased handle to a harness. Implements `Callable` so a
/// harness can be plugged in wherever an executable unit is expected
/// (workflow steps, team routing targets, tool slots).
pub struct HarnessRef {
    pub id: HarnessId,
    inner: Arc<dyn HarnessDispatch>,
}

impl HarnessRef {
    pub fn new(id: HarnessId, inner: Arc<dyn HarnessDispatch>) -> Self {
        Self { id, inner }
    }

    pub async fn run(&self) -> Result<Value> {
        self.inner.dispatch().await
    }
}

#[async_trait]
impl Callable for HarnessRef {
    async fn call(&self, _input: Value, _ctx: CallCtx) -> Result<Value> {
        self.run().await
    }

    fn label(&self) -> &str {
        self.id.as_str()
    }
}
