//! `ToolReturn` — richer return type for tools that need to drive
//! graph control flow or separate model-visible content from large
//! artifacts.
//!
//! The base `Tool` trait's `invoke` returns `Value` for backwards
//! compatibility. Tools that want richer behavior implement
//! `RichTool::invoke_rich` and the agent loop will pick it up via
//! the registry-side adapter.

use async_trait::async_trait;
use atomr_agents_core::{InvokeCtx, Result, Value};
use serde::{Deserialize, Serialize};

use crate::descriptor::ToolDescriptor;
use crate::r#trait::Tool;

/// What a richer tool returns. Agents map this back into the message
/// sequence and (optionally) the workflow state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolReturn {
    /// Plain content — model sees it as a `Role::Tool` message.
    Content(Value),
    /// Both model-visible content and an out-of-band artifact (e.g. a
    /// large blob). The artifact is stashed by the runner under a
    /// tool-named slot for later retrieval; only `content` enters the
    /// next prompt turn.
    ContentAndArtifact { content: Value, artifact: Value },
    /// Drive the surrounding harness/graph: send a control instruction.
    Command(ToolControl),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolControl {
    /// Hand off control to another agent / handler by id.
    Handoff { target: String, payload: Value },
    /// Terminate the current turn early with this value.
    Done(Value),
    /// Update one or more workflow channels.
    Update(Vec<(String, Value)>),
}

#[async_trait]
pub trait RichTool: Send + Sync + 'static {
    fn descriptor(&self) -> &ToolDescriptor;
    async fn invoke_rich(&self, args: Value, ctx: &InvokeCtx) -> Result<ToolReturn>;
}

/// Any `RichTool` is automatically a `Tool`: `invoke` projects
/// `ToolReturn::Content` (or the `content` field of
/// `ContentAndArtifact`); other variants surface as a synthetic
/// content carrying the control payload, so legacy callers keep
/// working.
#[async_trait]
impl<T: RichTool> Tool for T {
    fn descriptor(&self) -> &ToolDescriptor {
        RichTool::descriptor(self)
    }

    async fn invoke(&self, args: Value, ctx: &InvokeCtx) -> Result<Value> {
        match RichTool::invoke_rich(self, args, ctx).await? {
            ToolReturn::Content(v) => Ok(v),
            ToolReturn::ContentAndArtifact { content, .. } => Ok(content),
            ToolReturn::Command(c) => Ok(serde_json::to_value(c).unwrap_or(Value::Null)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::ToolSchema;
    use atomr_agents_core::{
        CallCtx, InvokeCtx, IterationBudget, MoneyBudget, TimeBudget, TokenBudget, ToolId,
    };
    use std::time::Duration;

    struct Handoff {
        d: ToolDescriptor,
    }

    #[async_trait]
    impl RichTool for Handoff {
        fn descriptor(&self) -> &ToolDescriptor {
            &self.d
        }
        async fn invoke_rich(&self, _args: Value, _ctx: &InvokeCtx) -> Result<ToolReturn> {
            Ok(ToolReturn::Command(ToolControl::Handoff {
                target: "specialist".into(),
                payload: serde_json::json!({"why": "complex"}),
            }))
        }
    }

    fn ictx() -> InvokeCtx {
        InvokeCtx {
            call: CallCtx {
                agent_id: None,
                tokens: TokenBudget::new(1000),
                time: TimeBudget::new(Duration::from_secs(5)),
                money: MoneyBudget::from_usd(0.10),
                iterations: IterationBudget::new(5),
                trace: vec![],
            },
            tool_call_id: "t1".into(),
            raw_args: Value::Null,
        }
    }

    #[tokio::test]
    async fn rich_tool_acts_as_plain_tool() {
        let t = Handoff {
            d: ToolDescriptor {
                id: ToolId::from("handoff"),
                name: "handoff".into(),
                description: "delegate to specialist".into(),
                schema: ToolSchema::empty_object(),
            },
        };
        let v = Tool::invoke(&t, Value::Null, &ictx()).await.unwrap();
        assert!(v.is_object()); // serialized ToolControl
    }

    #[tokio::test]
    async fn rich_invoke_returns_command() {
        let t = Handoff {
            d: ToolDescriptor {
                id: ToolId::from("handoff"),
                name: "handoff".into(),
                description: "delegate".into(),
                schema: ToolSchema::empty_object(),
            },
        };
        let r = t.invoke_rich(Value::Null, &ictx()).await.unwrap();
        assert!(matches!(r, ToolReturn::Command(ToolControl::Handoff { .. })));
    }
}
