//! Transcript node steps — auditable per-role log.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Which role produced this step.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Clarifier,
    Planner,
    Researcher,
    Writer,
    Critic,
    Verifier,
    Supervisor,
    #[default]
    Other,
}

/// One audit entry for the transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeStep {
    pub role: NodeKind,
    /// Human-readable label, e.g. `"planner"`, `"researcher:sq-2"`.
    pub label: String,
    /// Timestamp the step was recorded.
    pub ts: DateTime<Utc>,
    /// Short summary of what the role did.
    pub summary: String,
    /// Optional reference to the sub-question this step was working on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_question_id: Option<String>,
}

impl NodeStep {
    pub fn new(role: NodeKind, label: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            role,
            label: label.into(),
            ts: Utc::now(),
            summary: summary.into(),
            sub_question_id: None,
        }
    }
}
