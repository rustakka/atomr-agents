//! `update_action` — patch an existing action in place.

use async_trait::async_trait;
use atomr_agents_core::{InvokeCtx, Result as CoreResult, ToolId, Value};
use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};
use serde::Deserialize;
use serde_json::json;

use crate::analysis::ActionStatus;
use crate::error::MeetingsHarnessError;
use crate::tools::ToolHandle;

#[derive(Debug, Deserialize)]
struct Args {
    action_id: String,
    #[serde(default)]
    status: Option<ActionStatus>,
    #[serde(default)]
    owner_attendee_id: Option<String>,
    #[serde(default)]
    due_iso: Option<String>,
    #[serde(default)]
    supporting_quote: Option<String>,
}

pub struct UpdateActionTool {
    handle: ToolHandle,
    descriptor: ToolDescriptor,
}

impl UpdateActionTool {
    pub fn new(handle: ToolHandle) -> Self {
        let descriptor = ToolDescriptor {
            id: ToolId::from("meetings.update_action"),
            name: "update_action".into(),
            description: "Patch a single field of an existing action (status, owner, due, supporting_quote). Does not reorder.".into(),
            schema: ToolSchema(json!({
                "type": "object",
                "required": ["action_id"],
                "properties": {
                    "action_id": { "type": "string" },
                    "status": { "type": "string", "enum": ["open", "done", "cancelled"] },
                    "owner_attendee_id": { "type": "string" },
                    "due_iso": { "type": "string" },
                    "supporting_quote": { "type": "string" }
                }
            })),
        };
        Self { handle, descriptor }
    }
}

#[async_trait]
impl Tool for UpdateActionTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> CoreResult<Value> {
        let args: Args = serde_json::from_value(args).map_err(MeetingsHarnessError::from)?;
        self.handle
            .update_action(
                &args.action_id,
                args.status,
                args.owner_attendee_id,
                args.due_iso,
                args.supporting_quote,
            )
            .map_err(atomr_agents_core::AgentError::from)?;
        Ok(json!({ "ok": true }))
    }
}
