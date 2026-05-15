//! `finalize` — mark the analysis final and optionally set a TL;DR.

use async_trait::async_trait;
use atomr_agents_core::{InvokeCtx, Result as CoreResult, ToolId, Value};
use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};
use serde::Deserialize;
use serde_json::json;

use crate::error::MeetingsHarnessError;
use crate::tools::ToolHandle;

#[derive(Debug, Default, Deserialize)]
struct Args {
    #[serde(default)]
    tldr: Option<String>,
    #[serde(default = "default_reason")]
    reason: String,
}

fn default_reason() -> String {
    "agent_finalize".into()
}

pub struct FinalizeTool {
    handle: ToolHandle,
    descriptor: ToolDescriptor,
}

impl FinalizeTool {
    pub fn new(handle: ToolHandle) -> Self {
        let descriptor = ToolDescriptor {
            id: ToolId::from("meetings.finalize"),
            name: "finalize".into(),
            description:
                "Mark the analysis final, optionally setting the TL;DR. Signals the loop to terminate.".into(),
            schema: ToolSchema(json!({
                "type": "object",
                "properties": {
                    "tldr": { "type": "string" },
                    "reason": { "type": "string" }
                }
            })),
        };
        Self { handle, descriptor }
    }
}

#[async_trait]
impl Tool for FinalizeTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> CoreResult<Value> {
        let args: Args = if args.is_null() {
            Args {
                tldr: None,
                reason: default_reason(),
            }
        } else {
            serde_json::from_value(args).map_err(MeetingsHarnessError::from)?
        };
        let (notes, actions) = self.handle.finalize(args.reason.clone(), args.tldr);
        Ok(json!({ "note_count": notes, "action_count": actions }))
    }
}
