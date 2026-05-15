//! Intermediate artifacts produced during a run.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::request::Markdown;

/// One section of the running draft.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftSection {
    /// Outline heading this section belongs to.
    pub heading: String,
    pub body: Markdown,
    /// Sub-question ids the section answers.
    #[serde(default)]
    pub answers_sub_questions: Vec<String>,
}

/// Raw search hit recorded so the verifier and the UI can audit
/// citations after the fact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawSearchHit {
    pub provider: String,
    pub url: Url,
    pub title: String,
    pub snippet: String,
    pub source: String,
    pub captured_at: DateTime<Utc>,
    /// Sub-question id that motivated this hit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_question_id: Option<String>,
    /// Optional fetched page body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

/// Side-data persisted alongside the report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Artifacts {
    /// Per-section drafts before they are stitched into the final
    /// report. Useful for retry / debugging.
    #[serde(default)]
    pub drafts: Vec<DraftSection>,
    /// Free-form scratchpad used by the writer / critic.
    #[serde(default)]
    pub scratchpad: String,
    /// Every raw search hit recorded during the run.
    #[serde(default)]
    pub raw_search_hits: Vec<RawSearchHit>,
}
