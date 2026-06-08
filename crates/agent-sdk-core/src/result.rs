//! Terminal value produced by a run (mirrors the SDK's `ResultMessage`).

use serde::{Deserialize, Serialize};

/// Token + cache usage reported by the SDK.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageSummary {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
}

/// The SDK `ResultMessage`, normalized. `cost_usd` is the SDK's
/// **client-side estimate**, not an invoice.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResultSummary {
    /// `"success"`, `"error_max_turns"`, `"error_during_execution"`, …
    #[serde(default)]
    pub subtype: String,
    /// Final assistant text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// SDK conversation session id (for resume).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default)]
    pub num_turns: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub usage: UsageSummary,
    #[serde(default)]
    pub is_error: bool,
}

impl ResultSummary {
    /// Cost in integer micro-USD (0 when the SDK reported no estimate).
    pub fn cost_micro_usd(&self) -> u64 {
        self.cost_usd.map(|c| (c * 1_000_000.0) as u64).unwrap_or(0)
    }
}
