//! On-the-wire JSON shapes for the Tavily Search API.
//!
//! Only the fields we map are deserialised. Tavily occasionally adds
//! new top-level fields; `#[allow(dead_code)]` keeps the unused ones
//! around for future expansion without churn.

#![allow(dead_code)]

use atomr_agents_web_search_core::{WebSearchHit, WebSearchRequest};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

/// Maximum snippet length we synthesise from Tavily's `content` field
/// when the provider doesn't return a separate short description.
const SNIPPET_CAP: usize = 200;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct TavilyRequest<'a> {
    pub api_key: &'a str,
    pub query: &'a str,
    pub max_results: u32,
    pub search_depth: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_answer: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub include_domains: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exclude_domains: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days: Option<u32>,
}

impl<'a> TavilyRequest<'a> {
    pub fn build(
        api_key: &'a str,
        search_depth: &'a str,
        include_answer: bool,
        req: &'a WebSearchRequest,
        cap: u32,
    ) -> Self {
        let max_results = req.max_results.clamp(1, cap);
        Self {
            api_key,
            query: req.query.as_str(),
            max_results,
            search_depth,
            include_answer: if include_answer { Some(true) } else { None },
            include_domains: req.allowed_domains.clone(),
            exclude_domains: req.blocked_domains.clone(),
            days: req.recency_days,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct TavilyResponse {
    #[serde(default)]
    pub results: Vec<TavilyResult>,
    #[serde(default)]
    pub answer: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct TavilyResult {
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub score: Option<f32>,
    #[serde(default)]
    pub published_date: Option<String>,
}

impl TavilyResponse {
    /// Map Tavily hits into the provider-agnostic
    /// [`WebSearchHit`] shape. Drops rows whose `url` doesn't parse.
    pub fn into_hits(self) -> Vec<WebSearchHit> {
        self.results
            .into_iter()
            .filter_map(|r| {
                let url = Url::parse(&r.url).ok()?;
                let snippet = clip(&r.content, SNIPPET_CAP);
                let source = url.host_str().unwrap_or("").to_string();
                Some(WebSearchHit {
                    url,
                    title: r.title,
                    snippet,
                    published: parse_published(r.published_date.as_deref()),
                    source,
                    score: r.score,
                    content: if r.content.is_empty() {
                        None
                    } else {
                        Some(r.content)
                    },
                })
            })
            .collect()
    }
}

fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        // Slice on a char boundary near `max`.
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &s[..end])
    }
}

fn parse_published(raw: Option<&str>) -> Option<DateTime<Utc>> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    // Tavily uses ISO-8601; fall back to date-only.
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Some(dt.with_timezone(&Utc));
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        return d
            .and_hms_opt(0, 0, 0)
            .map(|n| DateTime::<Utc>::from_naive_utc_and_offset(n, Utc));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_serializes_with_api_key_in_body() {
        let req = WebSearchRequest::new("rust actor frameworks")
            .with_max_results(5)
            .with_recency_days(30);
        let wire = TavilyRequest::build("k-123", "basic", false, &req, 20);
        let v = serde_json::to_value(&wire).unwrap();
        assert_eq!(v["api_key"], "k-123");
        assert_eq!(v["query"], "rust actor frameworks");
        assert_eq!(v["max_results"], 5);
        assert_eq!(v["search_depth"], "basic");
        assert_eq!(v["days"], 30);
        // Empty domain lists are omitted.
        assert!(v.get("include_domains").is_none());
        assert!(v.get("exclude_domains").is_none());
        // include_answer flag is omitted when false.
        assert!(v.get("include_answer").is_none());
    }

    #[test]
    fn request_caps_max_results() {
        let req = WebSearchRequest::new("x").with_max_results(99);
        let wire = TavilyRequest::build("k", "basic", false, &req, 20);
        assert_eq!(wire.max_results, 20);
    }

    #[test]
    fn response_parses_and_maps_to_hits() {
        let body = r#"{
            "answer": "synthesised",
            "results": [
                {
                    "url": "https://example.com/a",
                    "title": "A title",
                    "content": "extract text",
                    "score": 0.91,
                    "published_date": "2024-01-15T10:00:00Z"
                },
                {
                    "url": "not-a-valid-url",
                    "title": "skip me",
                    "content": ""
                }
            ]
        }"#;
        let resp: TavilyResponse = serde_json::from_str(body).unwrap();
        let hits = resp.into_hits();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "A title");
        assert_eq!(hits[0].snippet, "extract text");
        assert_eq!(hits[0].content.as_deref(), Some("extract text"));
        assert_eq!(hits[0].source, "example.com");
        assert_eq!(hits[0].score, Some(0.91));
        assert!(hits[0].published.is_some());
    }

    #[test]
    fn clip_clips_on_char_boundary() {
        // 200 cap on an ASCII string longer than 200.
        let s = "x".repeat(250);
        let c = clip(&s, 200);
        assert_eq!(c.chars().count(), 201); // 200 chars + "…"
        assert!(c.ends_with('…'));
    }
}
