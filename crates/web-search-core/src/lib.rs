//! Provider-agnostic web-search trait + request / hit types.
//!
//! Concrete providers (Tavily, SerpAPI, DuckDuckGo, Brave, …) ship as
//! separate crates and implement [`WebSearch`]. The mock implementation
//! [`MockWebSearch`] is deterministic so unit tests and the deep-research
//! harness's integration tests run end-to-end without network access.

#![forbid(unsafe_code)]

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

/// Crate error type.
#[derive(Debug, Error)]
pub enum WebSearchError {
    /// The provider rejected the request (rate limit, bad query, etc.).
    #[error("provider error: {0}")]
    Provider(String),
    /// Transport / network failure underneath the provider.
    #[error("transport error: {0}")]
    Transport(String),
    /// Configuration error (missing api key, invalid endpoint).
    #[error("configuration error: {0}")]
    Config(String),
    /// Invalid input shape.
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    /// Catch-all.
    #[error("{0}")]
    Other(String),
}

pub type Result<T, E = WebSearchError> = std::result::Result<T, E>;

/// A query passed to a [`WebSearch`] provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchRequest {
    /// Free-text query the user / agent wants answers to.
    pub query: String,
    /// Soft cap on the number of returned hits. Providers may return
    /// fewer; they should not return more.
    #[serde(default = "default_max_results")]
    pub max_results: u32,
    /// Domain allow-list. Empty means "no restriction".
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    /// Domain deny-list. Applied after the allow-list.
    #[serde(default)]
    pub blocked_domains: Vec<String>,
    /// Restrict hits to the last `recency_days` days, if the provider
    /// supports filtering by date.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recency_days: Option<u32>,
    /// Optional locale hint (`en-US`, `de-DE`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    /// Optional safe-search hint. Providers without an equivalent
    /// concept may ignore this.
    #[serde(default)]
    pub safe_search: SafeSearch,
}

fn default_max_results() -> u32 {
    8
}

impl WebSearchRequest {
    /// Build a request with sensible defaults: 8 results, no filters.
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            max_results: default_max_results(),
            allowed_domains: Vec::new(),
            blocked_domains: Vec::new(),
            recency_days: None,
            locale: None,
            safe_search: SafeSearch::default(),
        }
    }

    pub fn with_max_results(mut self, n: u32) -> Self {
        self.max_results = n;
        self
    }

    pub fn with_allowed_domains<I, S>(mut self, domains: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.allowed_domains = domains.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_recency_days(mut self, days: u32) -> Self {
        self.recency_days = Some(days);
        self
    }
}

/// Safe-search hint.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeSearch {
    Off,
    #[default]
    Moderate,
    Strict,
}

/// One result row returned by a [`WebSearch`] provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchHit {
    pub url: Url,
    pub title: String,
    pub snippet: String,
    /// Provider-reported publication time, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published: Option<DateTime<Utc>>,
    /// Provider-reported source domain (eg `"nytimes.com"`). Computed
    /// from `url` when the provider doesn't supply one explicitly.
    pub source: String,
    /// Optional provider-supplied relevance score in `[0, 1]`. Mock
    /// providers leave this `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
    /// Pre-fetched page content the provider extracted. Many providers
    /// (Tavily, Brave) optionally return a markdown / cleaned-text
    /// extract; the deep-research harness uses this when available
    /// instead of issuing a follow-up fetch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

impl WebSearchHit {
    pub fn new(url: Url, title: impl Into<String>, snippet: impl Into<String>) -> Self {
        let source = url.host_str().unwrap_or("").to_string();
        Self {
            url,
            title: title.into(),
            snippet: snippet.into(),
            published: None,
            source,
            score: None,
            content: None,
        }
    }
}

/// The provider-agnostic search trait. Implementations cover Tavily /
/// SerpAPI / DuckDuckGo / Brave / etc.
#[async_trait]
pub trait WebSearch: Send + Sync + 'static {
    /// Run a query. Implementations should apply `allowed_domains` /
    /// `blocked_domains` / `recency_days` filters when the provider
    /// supports them; otherwise they fall back to post-filtering.
    async fn search(&self, req: &WebSearchRequest) -> Result<Vec<WebSearchHit>>;
    /// Human-readable provider name for telemetry.
    fn provider_name(&self) -> &str;
}

#[async_trait]
impl WebSearch for Box<dyn WebSearch> {
    async fn search(&self, req: &WebSearchRequest) -> Result<Vec<WebSearchHit>> {
        (**self).search(req).await
    }
    fn provider_name(&self) -> &str {
        (**self).provider_name()
    }
}

#[async_trait]
impl WebSearch for Arc<dyn WebSearch> {
    async fn search(&self, req: &WebSearchRequest) -> Result<Vec<WebSearchHit>> {
        (**self).search(req).await
    }
    fn provider_name(&self) -> &str {
        (**self).provider_name()
    }
}

/// One fixture row: a query substring needle plus the hits to return.
type MockRow = (String, Vec<WebSearchHit>);

/// In-memory deterministic provider. Tests and the deep-research harness
/// default to this so they run without network access.
///
/// The mock stores `(needle, hits)` pairs. A request matches a row when
/// `request.query` contains the needle (case-insensitively). Unmatched
/// queries return an empty list — production-style providers behave
/// the same way under "no results".
#[derive(Default, Clone)]
pub struct MockWebSearch {
    rows: Arc<RwLock<Vec<MockRow>>>,
}

impl MockWebSearch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a (query-substring, fixture-hits) pair. Multiple rows
    /// may match a single query; they are concatenated.
    pub fn with_fixture(self, needle: impl Into<String>, hits: Vec<WebSearchHit>) -> Self {
        self.rows.write().push((needle.into().to_lowercase(), hits));
        self
    }

    /// Number of fixture rows registered.
    pub fn fixture_count(&self) -> usize {
        self.rows.read().len()
    }
}

#[async_trait]
impl WebSearch for MockWebSearch {
    async fn search(&self, req: &WebSearchRequest) -> Result<Vec<WebSearchHit>> {
        let q = req.query.to_lowercase();
        let mut out: Vec<WebSearchHit> = Vec::new();
        let rows = self.rows.read();
        for (needle, hits) in rows.iter() {
            if q.contains(needle) {
                out.extend(hits.iter().cloned());
            }
        }
        out.retain(|h| domain_ok(&h.source, &req.allowed_domains, &req.blocked_domains));
        out.truncate(req.max_results as usize);
        Ok(out)
    }

    fn provider_name(&self) -> &str {
        "mock"
    }
}

fn domain_ok(source: &str, allowed: &[String], blocked: &[String]) -> bool {
    if !allowed.is_empty() && !allowed.iter().any(|d| source.ends_with(d.as_str())) {
        return false;
    }
    if blocked.iter().any(|d| source.ends_with(d.as_str())) {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(url: &str, title: &str) -> WebSearchHit {
        WebSearchHit::new(Url::parse(url).unwrap(), title, format!("snippet for {title}"))
    }

    #[tokio::test]
    async fn mock_returns_matching_fixture() {
        let mock = MockWebSearch::new().with_fixture(
            "rust",
            vec![
                hit("https://rust-lang.org/", "Rust homepage"),
                hit("https://blog.rust-lang.org/", "Rust blog"),
            ],
        );
        let req = WebSearchRequest::new("compare actor frameworks in rust");
        let hits = mock.search(&req).await.unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "Rust homepage");
        assert_eq!(hits[0].source, "rust-lang.org");
    }

    #[tokio::test]
    async fn mock_truncates_to_max_results() {
        let mock = MockWebSearch::new().with_fixture(
            "x",
            (0..10)
                .map(|i| hit(&format!("https://x.test/{i}"), &format!("t{i}")))
                .collect(),
        );
        let req = WebSearchRequest::new("xxx").with_max_results(3);
        let hits = mock.search(&req).await.unwrap();
        assert_eq!(hits.len(), 3);
    }

    #[tokio::test]
    async fn mock_respects_allow_and_block_lists() {
        let mock = MockWebSearch::new().with_fixture(
            "x",
            vec![
                hit("https://good.test/a", "ok-a"),
                hit("https://bad.test/a", "block-a"),
            ],
        );
        // allow-list: only good.test
        let hits = mock
            .search(&WebSearchRequest::new("xxx").with_allowed_domains(["good.test"]))
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "ok-a");
        // block-list: drop bad.test
        let mut req = WebSearchRequest::new("xxx");
        req.blocked_domains.push("bad.test".into());
        let hits = mock.search(&req).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "ok-a");
    }

    #[tokio::test]
    async fn mock_returns_empty_for_no_match() {
        let mock = MockWebSearch::new().with_fixture("foo", vec![hit("https://foo.test/", "foo")]);
        let req = WebSearchRequest::new("bar baz");
        let hits = mock.search(&req).await.unwrap();
        assert!(hits.is_empty());
    }
}
