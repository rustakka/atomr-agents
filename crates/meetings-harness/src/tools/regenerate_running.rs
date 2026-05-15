//! `regenerate_running` — recompute the running rollup.

use async_trait::async_trait;
use atomr_agents_core::{InvokeCtx, Result as CoreResult, ToolId, Value};
use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};
use serde_json::json;

use crate::tools::ToolHandle;

pub struct RegenerateRunningTool {
    handle: ToolHandle,
    descriptor: ToolDescriptor,
}

impl RegenerateRunningTool {
    pub fn new(handle: ToolHandle) -> Self {
        let descriptor = ToolDescriptor {
            id: ToolId::from("meetings.regenerate_running"),
            name: "regenerate_running".into(),
            description:
                "Recompute the running rollup `summary_levels.running` from finalized segments. The default rule concatenates segment texts; smarter rollups can overwrite directly."
                    .into(),
            schema: ToolSchema(json!({"type": "object", "properties": {}})),
        };
        Self { handle, descriptor }
    }
}

#[async_trait]
impl Tool for RegenerateRunningTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn invoke(&self, _args: Value, _ctx: &InvokeCtx) -> CoreResult<Value> {
        let text = self.handle.regenerate_running();
        Ok(json!({ "running": text }))
    }
}
