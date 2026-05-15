//! Coverage signals — how well the run covered the plan.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Per-section / per-sub-question coverage signals.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CoverageSignals {
    /// Number of sub-questions whose `status == Answered`.
    #[serde(default)]
    pub sub_questions_answered: u32,
    /// Number of sub-questions still pending or unresolved.
    #[serde(default)]
    pub sub_questions_unresolved: u32,
    /// Outline sections that the writer left empty.
    #[serde(default)]
    pub unresolved_gaps: Vec<String>,
    /// Heuristic confidence in `[0, 1]` per outline section.
    #[serde(default)]
    pub confidence_per_section: BTreeMap<String, f32>,
}
