//! `append_action` — append to the linear actions ledger.

use async_trait::async_trait;
use atomr_agents_core::{InvokeCtx, Result as CoreResult, ToolId, Value};
use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};
use serde::Deserialize;
use serde_json::json;

use crate::error::MeetingsHarnessError;
use crate::tools::ToolHandle;

#[derive(Debug, Deserialize)]
struct Args {
    description: String,
    #[serde(default)]
    owner_attendee_id: Option<String>,
    #[serde(default)]
    due_iso: Option<String>,
    #[serde(default)]
    supporting_quote: Option<String>,
    #[serde(default)]
    source_turn_index: Option<u64>,
}

pub struct AppendActionTool {
    handle: ToolHandle,
    descriptor: ToolDescriptor,
}

impl AppendActionTool {
    pub fn new(handle: ToolHandle) -> Self {
        let descriptor = ToolDescriptor {
            id: ToolId::from("meetings.append_action"),
            name: "append_action".into(),
            description: "Append an action item. `owner_attendee_id`, when supplied, must resolve to an existing attendee — call upsert_attendee first. Actions are append-only.".into(),
            schema: ToolSchema(json!({
                "type": "object",
                "required": ["description"],
                "properties": {
                    "description": { "type": "string", "minLength": 1 },
                    "owner_attendee_id": { "type": "string" },
                    "due_iso": { "type": "string" },
                    "supporting_quote": { "type": "string" },
                    "source_turn_index": { "type": "integer", "minimum": 0 }
                }
            })),
        };
        Self { handle, descriptor }
    }
}

#[async_trait]
impl Tool for AppendActionTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> CoreResult<Value> {
        let args: Args = serde_json::from_value(args).map_err(MeetingsHarnessError::from)?;
        let id = self
            .handle
            .append_action(
                args.description,
                args.owner_attendee_id,
                args.due_iso,
                args.supporting_quote,
                args.source_turn_index,
            )
            .map_err(atomr_agents_core::AgentError::from)?;
        Ok(json!({ "action_id": id }))
    }
}
