//! Scope: data sources, domain filters, attachments.

use serde::{Deserialize, Serialize};

/// Reference to a registered data source (retriever id, corpus id, …).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSourceRef {
    /// Stable id understood by the host. The harness looks it up in a
    /// registry of `Retriever`s.
    pub id: String,
    /// Free-text label for the UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Optional binary or text attachment to consider as evidence (e.g. an
/// uploaded PDF). The harness opens these through registered
/// `Retriever`s — this crate is metadata-only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentRef {
    pub id: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Scope for a research run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResearchScope {
    /// Registered data sources to query alongside the public web.
    #[serde(default)]
    pub data_sources: Vec<DataSourceRef>,
    /// Web-search domain allow-list. Empty means "no restriction".
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    /// Web-search domain block-list applied after `allowed_domains`.
    #[serde(default)]
    pub blocked_domains: Vec<String>,
    /// Optional attachments to treat as evidence.
    #[serde(default)]
    pub attachments: Vec<AttachmentRef>,
    /// Optional user-supplied background notes (e.g. paste of an
    /// existing draft). Roles may reference this when forming sub-
    /// questions or drafting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
}
