//! `HandoffTool` — built-in helper for multi-agent handoff patterns.
//!
//! Returns a `ToolReturn::Command(ToolControl::Handoff)` that downstream
//! routers (supervisor / swarm / network / hierarchical) interpret to
//! transfer control. Lives in `agents-tool::stdlib::handoff`.

use async_trait::async_trait;
use atomr_agents_core::{InvokeCtx, Result, ToolId, Value};

use crate::descriptor::{ToolDescriptor, ToolSchema};
use crate::tool_return::{RichTool, ToolControl, ToolReturn};

pub struct HandoffTool {
    pub default_target: String,
    descriptor: ToolDescriptor,
}

impl HandoffTool {
    pub fn new(default_target: impl Into<String>) -> Self {
        let target = default_target.into();
        Self {
            descriptor: ToolDescriptor {
                id: ToolId::from(format!("handoff_{target}")),
                name: format!("handoff_to_{target}"),
                description: format!("Hand off control to the {target} agent."),
                schema: ToolSchema(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "target": {"type": "string"},
                        "payload": {},
                    }
                })),
            },
            default_target: target,
        }
    }
}

#[async_trait]
impl RichTool for HandoffTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }
    async fn invoke_rich(&self, args: Value, _ctx: &InvokeCtx) -> Result<ToolReturn> {
        let target = args
            .get("target")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.default_target.clone());
        let payload = args.get("payload").cloned().unwrap_or(Value::Null);
        Ok(ToolReturn::Command(ToolControl::Handoff { target, payload }))
    }
}
