//! `append_note` — append to the linear notes ledger.

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
    #[serde(default)]
    source_turn_indices: Vec<u64>,
    #[serde(default)]
    start_ms: Option<u32>,
    #[serde(default)]
    end_ms: Option<u32>,
}

pub struct AppendNoteTool {
    handle: ToolHandle,
    descriptor: ToolDescriptor,
}

impl AppendNoteTool {
    pub fn new(handle: ToolHandle) -> Self {
        let descriptor = ToolDescriptor {
            id: ToolId::from("meetings.append_note"),
            name: "append_note".into(),
            description: "Append a note to the linear ledger. Notes are append-only; existing notes are never reordered or deleted. Returns the note's stable id.".into(),
            schema: ToolSchema(json!({
                "type": "object",
                "required": ["text"],
                "properties": {
                    "text": { "type": "string", "minLength": 1 },
                    "source_turn_indices": {
                        "type": "array",
                        "items": { "type": "integer", "minimum": 0 }
                    },
                    "start_ms": { "type": "integer", "minimum": 0 },
                    "end_ms": { "type": "integer", "minimum": 0 }
                }
            })),
        };
        Self { handle, descriptor }
    }
}

#[async_trait]
impl Tool for AppendNoteTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> CoreResult<Value> {
        let args: Args = serde_json::from_value(args).map_err(MeetingsHarnessError::from)?;
        let id = self
            .handle
            .append_note(args.text, args.source_turn_indices, args.start_ms, args.end_ms);
        Ok(json!({ "note_id": id }))
    }
}
