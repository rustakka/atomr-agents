//! Project a normalized [`AgentSdkMessage`] three ways: onto the broadcast
//! event stream, onto the core [`EventBus`], and onto the [`SpendLedger`].
//!
//! Returns `Some(ResultSummary)` on the terminal `Result` message.

use tokio::sync::broadcast;

use atomr_agents_agent::SpendLedger;
use atomr_agents_agent_sdk_core::{
    AgentSdkEvent, AgentSdkMessage, ContentBlock, FinishReason, ResultSummary,
};
use atomr_agents_core::{AgentId, Event, HarnessId};
use atomr_agents_observability::EventBus;

use crate::budget;

pub fn project(
    msg: &AgentSdkMessage,
    harness_id: &HarnessId,
    event_tx: &broadcast::Sender<AgentSdkEvent>,
    bus: &EventBus,
    ledger: &SpendLedger,
) -> Option<ResultSummary> {
    match msg {
        AgentSdkMessage::System {
            session_id,
            tools,
            mcp_servers,
            ..
        } => {
            let _ = event_tx.send(AgentSdkEvent::SystemInit {
                session_id: session_id.clone(),
                tools: tools.clone(),
                mcp_servers: mcp_servers.clone(),
            });
            None
        }
        AgentSdkMessage::Assistant { blocks } => {
            for b in blocks {
                let ev = match b {
                    ContentBlock::Text { text } => AgentSdkEvent::AssistantTextDelta { text: text.clone() },
                    ContentBlock::Thinking { text } => AgentSdkEvent::ThinkingDelta { text: text.clone() },
                    ContentBlock::ToolUse { id, name, input } => AgentSdkEvent::ToolUse {
                        tool_use_id: id.clone(),
                        name: name.clone(),
                        input: input.clone(),
                    },
                    ContentBlock::ToolResult { tool_use_id, is_error, .. } => AgentSdkEvent::ToolResult {
                        tool_use_id: tool_use_id.clone(),
                        is_error: *is_error,
                    },
                };
                let _ = event_tx.send(ev);
            }
            None
        }
        AgentSdkMessage::Result(r) => {
            budget::record_result(ledger, r);
            let _ = event_tx.send(AgentSdkEvent::Usage {
                input_tokens: r.usage.input_tokens,
                output_tokens: r.usage.output_tokens,
                cost_usd: r.cost_usd,
            });
            bus.emit(Event::AgentTurn {
                agent_id: AgentId::from(harness_id.as_str()),
                input_tokens: r.usage.input_tokens as u32,
                output_tokens: r.usage.output_tokens as u32,
                reasoning_tokens: 0,
                cached_tokens: r.usage.cache_read_input_tokens as u32,
                finish_reason: None,
                elapsed_ms: r.duration_ms.unwrap_or(0),
            });
            bus.emit(Event::HarnessIteration {
                harness_id: harness_id.clone(),
                iteration: r.num_turns as u64,
                outcome: r.subtype.clone(),
                budget_remaining_tokens: 0,
            });
            let reason = if r.is_error {
                FinishReason::Error
            } else {
                FinishReason::Completed
            };
            let _ = event_tx.send(AgentSdkEvent::RunFinished {
                reason,
                session_id: r.session_id.clone(),
                result_text: r.result.clone(),
            });
            Some(r.clone())
        }
        AgentSdkMessage::Unknown => None,
    }
}
