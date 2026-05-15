//! Object-safe dispatch trait + the public type-erased handle.
//!
//! Mirrors `atomr_agents_harness::HarnessDispatch` / `HarnessRef`. The
//! typed [`crate::SttHarness`] keeps the hot path monomorphized;
//! [`SttHarnessRef`] is the uniform handle that Python bindings,
//! registries, and workflow composition hold. It implements
//! [`Callable`] so an STT harness drops in wherever an executable unit
//! is expected.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::Callable;
use atomr_agents_core::{CallCtx, HarnessId, Result as CoreResult, Value};

/// Object-safe trait every STT harness implements. Implementors are
/// the typed [`crate::SttHarness`] and the erased
/// [`crate::BoxedSttHarness`].
#[async_trait]
pub trait SttHarnessDispatch: Send + Sync + 'static {
    /// Run the harness, returning the conversation as a JSON value.
    async fn dispatch(&self) -> CoreResult<Value>;
}

/// Public, type-erased handle to an STT harness.
#[derive(Clone)]
pub struct SttHarnessRef {
    pub id: HarnessId,
    inner: Arc<dyn SttHarnessDispatch>,
}

impl SttHarnessRef {
    pub fn new(id: HarnessId, inner: Arc<dyn SttHarnessDispatch>) -> Self {
        Self { id, inner }
    }

    /// Run the harness; the conversation comes back as a JSON value.
    pub async fn run(&self) -> CoreResult<Value> {
        self.inner.dispatch().await
    }
}

#[async_trait]
impl Callable for SttHarnessRef {
    async fn call(&self, _input: Value, _ctx: CallCtx) -> CoreResult<Value> {
        self.run().await
    }

    fn label(&self) -> &str {
        self.id.as_str()
    }
}
