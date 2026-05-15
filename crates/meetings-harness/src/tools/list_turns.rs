//! `list_turns` — paged listing of source transcript turns.

use async_trait::async_trait;
use atomr_agents_core::{InvokeCtx, Result as CoreResult, ToolId, Value};
use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};
use serde::Deserialize;
use serde_json::json;

use crate::error::MeetingsHarnessError;
use crate::tools::ToolHandle;

#[derive(Debug, Default, Deserialize)]
struct Args {
    /// Inclusive start. Defaults to 0.
    #[serde(default)]
    since_index: Option<u64>,
    /// Page size. Defaults to all remaining.
    #[serde(default)]
    limit: Option<usize>,
}

pub struct ListTurnsTool {
    handle: ToolHandle,
    descriptor: ToolDescriptor,
}

impl ListTurnsTool {
    pub fn new(handle: ToolHandle) -> Self {
        let descriptor = ToolDescriptor {
            id: ToolId::from("meetings.list_turns"),
            name: "list_turns".into(),
            description: "List source transcript turns as (index, speaker_label, start_ms, end_ms, text) rows. Supports `since_index` for live mode and `limit` for paging.".into(),
            schema: ToolSchema(json!({
                "type": "object",
                "properties": {
                    "since_index": { "type": "integer", "minimum": 0 },
                    "limit": { "type": "integer", "minimum": 1 }
                }
            })),
        };
        Self { handle, descriptor }
    }
}

#[async_trait]
impl Tool for ListTurnsTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> CoreResult<Value> {
        let args: Args = serde_json::from_value(args).map_err(MeetingsHarnessError::from)?;
        let conv = self.handle.transcript_snapshot();
        let start = args.since_index.unwrap_or(0);
        let limit = args.limit.unwrap_or(usize::MAX);
        let rows: Vec<_> = conv
            .turns
            .iter()
            .filter(|t| t.index >= start)
            .take(limit)
            .map(|t| {
                let label = t
                    .speaker_id()
                    .map(|sid| conv.effective_label(sid))
                    .unwrap_or_else(|| "unknown".into());
                json!({
                    "index": t.index,
                    "speaker_id": t.speaker_id(),
                    "speaker_label": label,
                    "start_ms": t.start_ms,
                    "end_ms": t.end_ms,
                    "text": t.text,
                })
            })
            .collect();
        Ok(json!({ "turns": rows }))
    }
}
