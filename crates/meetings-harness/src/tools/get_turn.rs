//! `get_turn` — full detail for one turn.

use async_trait::async_trait;
use atomr_agents_core::{InvokeCtx, Result as CoreResult, ToolId, Value};
use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};
use serde::Deserialize;
use serde_json::json;

use crate::error::MeetingsHarnessError;
use crate::tools::ToolHandle;

#[derive(Debug, Deserialize)]
struct Args {
    turn_index: u64,
}

pub struct GetTurnTool {
    handle: ToolHandle,
    descriptor: ToolDescriptor,
}

impl GetTurnTool {
    pub fn new(handle: ToolHandle) -> Self {
        let descriptor = ToolDescriptor {
            id: ToolId::from("meetings.get_turn"),
            name: "get_turn".into(),
            description: "Return full detail (including word-level breakdown when available) for one turn by index.".into(),
            schema: ToolSchema(json!({
                "type": "object",
                "required": ["turn_index"],
                "properties": {
                    "turn_index": { "type": "integer", "minimum": 0 }
                }
            })),
        };
        Self { handle, descriptor }
    }
}

#[async_trait]
impl Tool for GetTurnTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> CoreResult<Value> {
        let args: Args = serde_json::from_value(args).map_err(MeetingsHarnessError::from)?;
        let conv = self.handle.transcript_snapshot();
        let Some(turn) = conv.turns.iter().find(|t| t.index == args.turn_index) else {
            return Err(MeetingsHarnessError::tool(format!(
                "unknown turn_index `{}`",
                args.turn_index
            ))
            .into());
        };
        let speaker_label = turn
            .speaker_id()
            .map(|sid| conv.effective_label(sid))
            .unwrap_or_else(|| "unknown".into());
        Ok(json!({
            "index": turn.index,
            "speaker_id": turn.speaker_id(),
            "speaker_label": speaker_label,
            "start_ms": turn.start_ms,
            "end_ms": turn.end_ms,
            "text": turn.text,
            "words": turn.words,
            "confidence": turn.confidence,
        }))
    }
}
