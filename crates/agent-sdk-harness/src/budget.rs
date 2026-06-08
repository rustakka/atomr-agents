//! Map SDK cost/usage into atomr's spend ledger.
//!
//! The SDK reports real spend on every `ResultMessage`. We charge it
//! directly to a [`SpendLedger`] (the `cost_usd` is the SDK's client-side
//! estimate; for Bedrock/Vertex it may be absent, in which case only the
//! token totals are recorded).

use atomr_agents_agent::{Spend, SpendLedger};
use atomr_agents_agent_sdk_core::ResultSummary;

/// Record one result's cost + tokens onto the ledger.
pub fn record_result(ledger: &SpendLedger, r: &ResultSummary) {
    let tokens = (r.usage.input_tokens + r.usage.output_tokens) as u32;
    ledger.record(&Spend {
        micro_usd: r.cost_micro_usd(),
        tokens,
        decision_key: None,
    });
}
