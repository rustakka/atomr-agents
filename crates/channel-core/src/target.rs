use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::CallableHandle;

use crate::content::{InboundMessage, MessageContent};
use crate::error::Result;

/// What a thread invokes when an inbound message arrives.
///
/// `Callable` works for [`AgentRef`](atomr_agents_core), `Team`,
/// `WorkflowStep`, plain [`FnCallable`](atomr_agents_callable::FnCallable)
/// — anything that already implements the trait.
///
/// `Harness` is special: [`HarnessRef::call`](atomr_agents_callable)
/// ignores its input, so binding a harness via raw `Callable` would
/// silently drop every inbound message. Instead, the orchestrator
/// applies the inbound through a user-supplied [`HarnessInputAdapter`]
/// (which mutates whatever state the harness reads on its next run)
/// and then optionally triggers `harness.run()`.
#[derive(Clone)]
pub enum ThreadTarget {
    Callable(CallableHandle),
    Harness {
        callable: CallableHandle,
        adapter: Arc<dyn HarnessInputAdapter>,
    },
}

impl ThreadTarget {
    pub fn callable(handle: CallableHandle) -> Self {
        Self::Callable(handle)
    }

    pub fn harness(callable: CallableHandle, adapter: Arc<dyn HarnessInputAdapter>) -> Self {
        Self::Harness { callable, adapter }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Callable(c) => c.label(),
            Self::Harness { callable, .. } => callable.label(),
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Callable(_) => "callable",
            Self::Harness { .. } => "harness",
        }
    }
}

impl std::fmt::Debug for ThreadTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThreadTarget")
            .field("kind", &self.kind())
            .field("label", &self.label())
            .finish()
    }
}

/// Bridge an inbound channel message into the state a harness reads on
/// its next run.
///
/// Example: a meetings harness that reads from an STT
/// `ConversationStore`. The adapter would append the inbound's text as
/// a new turn under the harness's source `conversation_id`, then return
/// — the orchestrator will trigger `harness.run()` next.
#[async_trait]
pub trait HarnessInputAdapter: Send + Sync + 'static {
    async fn apply(&self, msg: &InboundMessage) -> Result<()>;

    /// If `true` (default), the orchestrator runs the harness once per
    /// inbound. If `false`, the harness is expected to be driven
    /// externally; the orchestrator only calls `apply`.
    fn one_shot(&self) -> bool {
        true
    }

    /// Map the harness's `run()` result into an outbound reply. Default
    /// implementation extracts a `"text"` field if present.
    fn reply_from_result(&self, value: &serde_json::Value) -> Option<MessageContent> {
        if let Some(s) = value.get("text").and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return Some(MessageContent::text(s));
            }
        }
        if let Some(s) = value.as_str() {
            if !s.is_empty() {
                return Some(MessageContent::text(s));
            }
        }
        None
    }
}
