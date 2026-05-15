//! Terminal value produced by a headless run.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::event::FinishReason;
use crate::request::CliRunId;
use crate::vendor::CliVendorKind;

/// One tool the CLI invoked during the run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub tool_call_id: String,
    pub name: String,
    pub input: serde_json::Value,
    pub output: Option<serde_json::Value>,
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// Token + cost totals.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageSummary {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: Option<f64>,
}

impl UsageSummary {
    pub fn add(&mut self, input_tokens: u64, output_tokens: u64, cost_usd: Option<f64>) {
        self.input_tokens += input_tokens;
        self.output_tokens += output_tokens;
        if let Some(c) = cost_usd {
            *self.cost_usd.get_or_insert(0.0) += c;
        }
    }
}

/// Final shape of a headless run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliResult {
    pub run_id: CliRunId,
    pub vendor: CliVendorKind,

    /// Concatenation of all `AssistantTextDelta` events.
    pub final_text: String,

    /// Optional structured payload if the vendor produced one
    /// (`claude --json-schema`, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_output: Option<serde_json::Value>,

    pub tool_calls: Vec<ToolCallRecord>,
    pub usage: UsageSummary,

    pub finish_reason: FinishReason,
    pub exit_code: Option<i32>,

    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

impl CliResult {
    pub fn new(run_id: CliRunId, vendor: CliVendorKind) -> Self {
        Self {
            run_id,
            vendor,
            final_text: String::new(),
            structured_output: None,
            tool_calls: Vec::new(),
            usage: UsageSummary::default(),
            finish_reason: FinishReason::Completed,
            exit_code: None,
            started_at: Utc::now(),
            ended_at: None,
        }
    }
}
