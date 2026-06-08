//! Drive a one-shot query's message stream to its terminal result.

use futures::StreamExt;
use tokio::sync::broadcast;

use atomr_agents_agent::SpendLedger;
use atomr_agents_agent_sdk_core::{AgentSdkEvent, MessageStream, ResultSummary};
use atomr_agents_core::HarnessId;
use atomr_agents_observability::EventBus;

use crate::bridge;
use crate::error::{HarnessError, Result};

/// Consume `stream`, projecting each message and capturing the terminal
/// result. Enforces `max_cost_usd` at turn boundaries.
pub async fn drive(
    mut stream: MessageStream,
    harness_id: &HarnessId,
    event_tx: &broadcast::Sender<AgentSdkEvent>,
    bus: &EventBus,
    ledger: &SpendLedger,
    max_cost_usd: Option<f64>,
) -> Result<ResultSummary> {
    let cap_micro = max_cost_usd.map(|c| (c * 1_000_000.0) as u64);
    let mut result: Option<ResultSummary> = None;

    while let Some(item) = stream.next().await {
        let msg = item?; // AgentSdkError -> HarnessError::Sdk
        if let Some(r) = bridge::project(&msg, harness_id, event_tx, bus, ledger) {
            result = Some(r);
        }
        if let Some(cap) = cap_micro {
            if ledger.total_micro_usd() > cap {
                let _ = event_tx.send(AgentSdkEvent::RunFinished {
                    reason: atomr_agents_agent_sdk_core::FinishReason::BudgetExhausted,
                    session_id: result.as_ref().and_then(|r| r.session_id.clone()),
                    result_text: result.as_ref().and_then(|r| r.result.clone()),
                });
                return Err(HarnessError::Budget("money"));
            }
        }
    }

    result.ok_or(HarnessError::StreamClosed)
}
