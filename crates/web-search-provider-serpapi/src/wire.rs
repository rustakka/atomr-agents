//! On-the-wire shapes for SerpAPI. The request is a GET with query
//! string, so the "request" side is a small helper that builds a
//! `Vec<(&str, String)>` for `reqwest::Client::get(...).query(...)`.
//! The response side is a serde-derived view onto the JSON envelope.

#![allow(dead_code)]

use atomr_agents_web_search_core::{WebSearchHit, WebSearchRequest};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use url::Url;

/// Build the `?...` query string. The `api_key` is appended last so a
/// debug-print of the partially-built vector never leaks the secret
/// before the call site has a chance to scrub it.
pub(crate) fn build_query(
    api_key: &str,
    engine: &str,
    req: &WebSearchRequest,
    cap: u32,
) -> Vec<(&'static str, String)> {
    let max_results = req.max_results.clamp(1, cap);
    let mut q: Vec<(&'static str, String)> = Vec::with_capacity(8);
    q.push(("engine", engine.to_string()));
    q.push(("q", req.query.clone()));
    q.push(("num", max_results.to_string()));
    if let Some(days) = req.recency_days {
        if let Some(qdr) = qdr_for_days(days) {
            q.push(("tbs", format!("qdr:{qdr}")));
        }
    }
    if let Some(loc) = req.locale.as_deref() {
        q.push(("hl", loc.to_string()));
    }
    q.push(("api_key", api_key.to_string()));
    q
}

/// Map a recency window in days to Google's `tbs=qdr:<x>` knob.
/// Returns `None` for `0` and anything larger than a year — SerpAPI
/// then returns the full index, matching the contract of
/// `WebSearchRequest::recency_days`.
pub(crate) fn qdr_for_days(days: u32) -> Option<&'static str> {
    match days {
        0 => None,
        1 => Some("d"),
        2..=7 => Some("w"),
        8..=30 => Some("m"),
        31..=365 => Some("y"),
        _ => None,
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SerpApiResponse {
    #[serde(default)]
    pub organic_results: Vec<OrganicResult>,
    #[serde(default)]
    pub search_metadata: Option<SearchMetadata>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct OrganicResult {
    pub link: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub snippet: String,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub position: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct SearchMetadata {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub processed_at: Option<String>,
}

impl SerpApiResponse {
    pub fn into_hits(self) -> Vec<WebSearchHit> {
        self.organic_results
            .into_iter()
            .filter_map(|r| {
                let url = Url::parse(&r.link).ok()?;
                let source = url.host_str().unwrap_or("").to_string();
                Some(WebSearchHit {
                    url,
                    title: r.title,
                    snippet: r.snippet,
                    published: parse_published(r.date.as_deref()),
                    source,
                    score: None,
                    content: None,
                })
            })
            .collect()
    }
}

fn parse_published(raw: Option<&str>) -> Option<DateTime<Utc>> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Some(dt.with_timezone(&Utc));
    }
    // SerpAPI usually returns relative strings ("3 days ago") or
    // `MMM d, YYYY`. We try the absolute form; fail-soft otherwise.
    if let Ok(d) = chrono::NaiveDate::parse_from_str(raw, "%b %d, %Y") {
        return d
            .and_hms_opt(0, 0, 0)
            .map(|n| DateTime::<Utc>::from_naive_utc_and_offset(n, Utc));
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
    fn query_string_carries_key_and_recency() {
        let req = WebSearchRequest::new("rust async runtime")
            .with_max_results(7)
            .with_recency_days(14);
        let q = build_query("k-123", "google", &req, 100);
        let map: std::collections::HashMap<&str, String> = q.into_iter().collect();
        assert_eq!(map.get("engine"), Some(&"google".to_string()));
        assert_eq!(map.get("q"), Some(&"rust async runtime".to_string()));
        assert_eq!(map.get("num"), Some(&"7".to_string()));
        assert_eq!(map.get("tbs"), Some(&"qdr:m".to_string()));
        assert_eq!(map.get("api_key"), Some(&"k-123".to_string()));
    }

    #[test]
    fn qdr_thresholds() {
        assert_eq!(qdr_for_days(0), None);
        assert_eq!(qdr_for_days(1), Some("d"));
        assert_eq!(qdr_for_days(7), Some("w"));
        assert_eq!(qdr_for_days(30), Some("m"));
        assert_eq!(qdr_for_days(365), Some("y"));
        assert_eq!(qdr_for_days(366), None);
    }

    #[test]
    fn response_maps_to_hits() {
        let body = r#"{
            "search_metadata": { "status": "Success" },
            "organic_results": [
                {
                    "link": "https://rust-lang.org/",
                    "title": "Rust",
                    "snippet": "A language empowering everyone to build reliable and efficient software.",
                    "date": "Jan 12, 2024",
                    "position": 1
                },
                {
                    "link": "not-valid-url",
                    "title": "bad",
                    "snippet": ""
                }
            ]
        }"#;
        let resp: SerpApiResponse = serde_json::from_str(body).unwrap();
        let hits = resp.into_hits();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].source, "rust-lang.org");
        assert!(hits[0].published.is_some());
        assert!(hits[0].score.is_none());
        assert!(hits[0].content.is_none());
    }
}
