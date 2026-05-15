//! `revise_tail_segment` — rewrite the in-flight tail segment summary.

use async_trait::async_trait;
use atomr_agents_core::{InvokeCtx, Result as CoreResult, ToolId, Value};
use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};
use serde::Deserialize;
use serde_json::json;

use crate::error::MeetingsHarnessError;
use crate::tools::ToolHandle;

#[derive(Debug, Deserialize)]
struct Args {
    text: String,
    start_turn_index: u64,
    end_turn_index: u64,
}

pub struct ReviseTailSegmentTool {
    handle: ToolHandle,
    descriptor: ToolDescriptor,
}

impl ReviseTailSegmentTool {
    pub fn new(handle: ToolHandle) -> Self {
        let descriptor = ToolDescriptor {
            id: ToolId::from("meetings.revise_tail_segment"),
            name: "revise_tail_segment".into(),
            description: "Rewrite the in-flight (non-finalized) tail segment summary, or open one if none exists. Earlier, finalized segments are immutable.".into(),
            schema: ToolSchema(json!({
                "type": "object",
                "required": ["text", "start_turn_index", "end_turn_index"],
                "properties": {
                    "text": { "type": "string", "minLength": 1 },
                    "start_turn_index": { "type": "integer", "minimum": 0 },
                    "end_turn_index": { "type": "integer", "minimum": 0 }
                }
            })),
        };
        Self { handle, descriptor }
    }
}

#[async_trait]
impl Tool for ReviseTailSegmentTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> CoreResult<Value> {
        let args: Args = serde_json::from_value(args).map_err(MeetingsHarnessError::from)?;
        let seg = self
            .handle
            .revise_tail_segment(args.text, args.start_turn_index, args.end_turn_index)
            .map_err(atomr_agents_core::AgentError::from)?;
        Ok(json!({ "segment": seg }))
    }
}
