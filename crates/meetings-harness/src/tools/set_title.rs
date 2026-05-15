//! `set_title` — set or replace the meeting title.

use async_trait::async_trait;
use atomr_agents_core::{InvokeCtx, Result as CoreResult, ToolId, Value};
use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};
use serde::Deserialize;
use serde_json::json;

use crate::error::MeetingsHarnessError;
use crate::tools::ToolHandle;

#[derive(Debug, Deserialize)]
struct Args {
    title: String,
}

pub struct SetTitleTool {
    handle: ToolHandle,
    descriptor: ToolDescriptor,
}

impl SetTitleTool {
    pub fn new(handle: ToolHandle) -> Self {
        let descriptor = ToolDescriptor {
            id: ToolId::from("meetings.set_title"),
            name: "set_title".into(),
            description: "Set or replace the meeting title.".into(),
            schema: ToolSchema(json!({
                "type": "object",
                "required": ["title"],
                "properties": { "title": { "type": "string", "minLength": 1 } }
            })),
        };
        Self { handle, descriptor }
    }
}

#[async_trait]
impl Tool for SetTitleTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> CoreResult<Value> {
        let args: Args = serde_json::from_value(args).map_err(MeetingsHarnessError::from)?;
        self.handle.set_title(args.title);
        Ok(json!({ "ok": true }))
    }
}
