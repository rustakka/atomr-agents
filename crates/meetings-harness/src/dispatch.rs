//! Object-safe dispatch trait + the public type-erased handle.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::Callable;
use atomr_agents_core::{CallCtx, HarnessId, Result as CoreResult, Value};

use crate::error::MeetingsHarnessError;
use crate::harness::extract_conversation_id;

/// Object-safe trait every meetings harness implements.
#[async_trait]
pub trait MeetingsHarnessDispatch: Send + Sync + 'static {
    /// Run the harness for the given source `conversation_id`.
    async fn dispatch(&self, conversation_id: &str) -> CoreResult<Value>;
}

/// Public, type-erased handle.
#[derive(Clone)]
pub struct MeetingsHarnessRef {
    pub id: HarnessId,
    inner: Arc<dyn MeetingsHarnessDispatch>,
}

impl MeetingsHarnessRef {
    pub fn new(id: HarnessId, inner: Arc<dyn MeetingsHarnessDispatch>) -> Self {
        Self { id, inner }
    }

    /// Run the harness; the analysis comes back as a JSON value.
    pub async fn run(&self, conversation_id: &str) -> CoreResult<Value> {
        self.inner.dispatch(conversation_id).await
    }
}

#[async_trait]
impl Callable for MeetingsHarnessRef {
    async fn call(&self, input: Value, _ctx: CallCtx) -> CoreResult<Value> {
        let id = extract_conversation_id(&input).map_err(MeetingsHarnessError::Config)?;
        self.run(&id).await
    }

    fn label(&self) -> &str {
        self.id.as_str()
    }
}
