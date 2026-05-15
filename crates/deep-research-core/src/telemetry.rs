//! Telemetry attached to a run.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Telemetry for a single role / node.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeTelemetry {
    /// Role label (`clarifier`, `planner`, `researcher`, …).
    pub role: String,
    /// Tokens reported by the model provider for this role.
    #[serde(default)]
    pub tokens: u64,
    /// Number of tool calls issued by this role.
    #[serde(default)]
    pub tool_calls: u32,
    /// Wall-clock milliseconds.
    #[serde(default)]
    pub wall_ms: u64,
    /// USD cost estimate from the provider, if known.
    #[serde(default)]
    pub cost_usd: f64,
}

/// Run-level telemetry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Telemetry {
    /// Total tokens across every node.
    #[serde(default)]
    pub tokens: u64,
    /// Total tool calls across every node.
    #[serde(default)]
    pub tool_calls: u32,
    /// Total wall-clock milliseconds.
    #[serde(default)]
    pub wall_ms: u64,
    /// Total USD cost.
    #[serde(default)]
    pub cost_usd: f64,
    /// Per-node breakdown keyed by node label.
    #[serde(default)]
    pub per_node: BTreeMap<String, NodeTelemetry>,
}

impl Telemetry {
    /// Roll a single node into the totals (also recorded in `per_node`).
    pub fn accumulate(&mut self, label: impl Into<String>, node: NodeTelemetry) {
        self.tokens += node.tokens;
        self.tool_calls += node.tool_calls;
        self.wall_ms += node.wall_ms;
        self.cost_usd += node.cost_usd;
        self.per_node.insert(label.into(), node);
    }
}
