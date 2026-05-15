//! `finalize_segment` — freeze the current tail segment.

use async_trait::async_trait;
use atomr_agents_core::{InvokeCtx, Result as CoreResult, ToolId, Value};
use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};
use serde_json::json;

use crate::tools::ToolHandle;

pub struct FinalizeSegmentTool {
    handle: ToolHandle,
    descriptor: ToolDescriptor,
}

impl FinalizeSegmentTool {
    pub fn new(handle: ToolHandle) -> Self {
        let descriptor = ToolDescriptor {
            id: ToolId::from("meetings.finalize_segment"),
            name: "finalize_segment".into(),
            description: "Mark the in-flight tail segment finalized. After this, the next call to revise_tail_segment opens a fresh segment.".into(),
            schema: ToolSchema(json!({"type": "object", "properties": {}})),
        };
        Self { handle, descriptor }
    }
}

#[async_trait]
impl Tool for FinalizeSegmentTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn invoke(&self, _args: Value, _ctx: &InvokeCtx) -> CoreResult<Value> {
        let seg = self
            .handle
            .finalize_segment()
            .map_err(atomr_agents_core::AgentError::from)?;
        Ok(json!({ "finalized": seg }))
    }
}
