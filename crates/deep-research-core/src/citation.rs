//! Numbered citations attached to a report.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

/// Verification status of a citation after the verifier role runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CitationStatus {
    /// Not yet verified.
    #[default]
    Unverified,
    /// Verifier confirmed URL + snippet match.
    Verified,
    /// Verifier flagged: e.g. dead link, no matching content.
    Flagged,
}

/// A numbered citation referenced from the final report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
    /// 1-based index used in the report's markers, e.g. `[1]`.
    pub number: u32,
    pub url: Url,
    pub title: String,
    pub snippet: String,
    /// Domain (host portion of `url`), denormalized for filters.
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published: Option<DateTime<Utc>>,
    /// Sub-question ids this citation supports, if known.
    #[serde(default)]
    pub supports: Vec<String>,
    #[serde(default)]
    pub status: CitationStatus,
}

impl Citation {
    pub fn new(number: u32, url: Url, title: impl Into<String>, snippet: impl Into<String>) -> Self {
        let source = url.host_str().unwrap_or("").to_string();
        Self {
            number,
            url,
            title: title.into(),
            snippet: snippet.into(),
            source,
            published: None,
            supports: Vec::new(),
            status: CitationStatus::Unverified,
        }
    }
}
