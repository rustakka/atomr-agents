//! `upsert_attendee` — idempotent attendee add/merge.

use async_trait::async_trait;
use atomr_agents_core::{InvokeCtx, Result as CoreResult, ToolId, Value};
use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};
use serde::Deserialize;
use serde_json::json;

use crate::error::MeetingsHarnessError;
use crate::tools::ToolHandle;

#[derive(Debug, Deserialize)]
struct Args {
    display_name: String,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    speaker_tags: Vec<u8>,
    #[serde(default)]
    email: Option<String>,
}

pub struct UpsertAttendeeTool {
    handle: ToolHandle,
    descriptor: ToolDescriptor,
}

impl UpsertAttendeeTool {
    pub fn new(handle: ToolHandle) -> Self {
        let descriptor = ToolDescriptor {
            id: ToolId::from("meetings.upsert_attendee"),
            name: "upsert_attendee".into(),
            description:
                "Insert or merge an attendee. Matches existing attendees by display_name (case-insensitive) or any overlapping speaker_tag. Returns the attendee's stable id."
                    .into(),
            schema: ToolSchema(json!({
                "type": "object",
                "required": ["display_name"],
                "properties": {
                    "display_name": { "type": "string", "minLength": 1 },
                    "role": { "type": "string" },
                    "speaker_tags": {
                        "type": "array",
                        "items": { "type": "integer", "minimum": 0, "maximum": 255 }
                    },
                    "email": { "type": "string" }
                }
            })),
        };
        Self { handle, descriptor }
    }
}

#[async_trait]
impl Tool for UpsertAttendeeTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> CoreResult<Value> {
        let args: Args = serde_json::from_value(args).map_err(MeetingsHarnessError::from)?;
        let id = self
            .handle
            .upsert_attendee(args.display_name, args.role, args.speaker_tags, args.email);
        Ok(json!({ "attendee_id": id }))
    }
}
