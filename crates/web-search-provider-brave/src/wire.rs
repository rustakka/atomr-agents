//! On-the-wire shapes for the Brave Search API.

#![allow(dead_code)]

use atomr_agents_web_search_core::{WebSearchHit, WebSearchRequest};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use url::Url;

/// Build the query-string vector for a Brave request. Auth happens
/// via header, so the key never appears in the URL.
pub(crate) fn build_query(
    req: &WebSearchRequest,
    cap: u32,
    default_country: Option<&str>,
) -> Vec<(&'static str, String)> {
    let count = req.max_results.clamp(1, cap);
    let mut q: Vec<(&'static str, String)> = Vec::with_capacity(6);
    q.push(("q", req.query.clone()));
    q.push(("count", count.to_string()));
    if let Some(days) = req.recency_days {
        if let Some(f) = freshness_for_days(days) {
            q.push(("freshness", f.to_string()));
        }
    }
    if let Some(country) = country_code(req, default_country) {
        q.push(("country", country));
    }
    q
}

pub(crate) fn freshness_for_days(days: u32) -> Option<&'static str> {
    match days {
        0 => None,
        1 => Some("pd"),
        2..=7 => Some("pw"),
        8..=30 => Some("pm"),
        31..=365 => Some("py"),
        _ => None,
    }
}

fn country_code(req: &WebSearchRequest, default_country: Option<&str>) -> Option<String> {
    if let Some(locale) = &req.locale {
        // `en-US` → `US`; `de` → `DE` (Brave accepts both forms).
        if let Some((_, region)) = locale.split_once('-') {
            return Some(region.to_uppercase());
        }
        return Some(locale.to_uppercase());
    }
    default_country.map(|s| s.to_uppercase())
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct BraveResponse {
    #[serde(default)]
    pub web: Option<WebSection>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct WebSection {
    #[serde(default)]
    pub results: Vec<WebResult>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct WebResult {
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub age: Option<String>,
    #[serde(default)]
    pub page_age: Option<String>,
}

impl BraveResponse {
    pub fn into_hits(self) -> Vec<WebSearchHit> {
        let Some(web) = self.web else {
            return Vec::new();
        };
        web.results
            .into_iter()
            .filter_map(|r| {
                let url = Url::parse(&r.url).ok()?;
                let source = url.host_str().unwrap_or("").to_string();
                let published = parse_published(r.page_age.as_deref());
                Some(WebSearchHit {
                    url,
                    title: r.title,
                    snippet: r.description,
                    published,
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
    fn query_builder_omits_country_when_none() {
        let req = WebSearchRequest::new("rust");
        let q = build_query(&req, 20, None);
        let map: std::collections::HashMap<&str, String> = q.into_iter().collect();
        assert_eq!(map.get("q"), Some(&"rust".to_string()));
        assert_eq!(map.get("count"), Some(&"8".to_string()));
        assert!(!map.contains_key("country"));
        assert!(!map.contains_key("freshness"));
    }

    #[test]
    fn locale_overrides_default_country() {
        let mut req = WebSearchRequest::new("rust");
        req.locale = Some("en-GB".into());
        let q = build_query(&req, 20, Some("us"));
        let map: std::collections::HashMap<&str, String> = q.into_iter().collect();
        assert_eq!(map.get("country"), Some(&"GB".to_string()));
    }

    #[test]
    fn recency_maps_to_freshness() {
        let req = WebSearchRequest::new("rust").with_recency_days(14);
        let q = build_query(&req, 20, None);
        let map: std::collections::HashMap<&str, String> = q.into_iter().collect();
        assert_eq!(map.get("freshness"), Some(&"pm".to_string()));
    }

    #[test]
    fn response_maps_to_hits() {
        let body = r#"{
            "web": {
                "results": [
                    {
                        "url": "https://rust-lang.org/",
                        "title": "Rust",
                        "description": "A language ...",
                        "age": "2 weeks ago",
                        "page_age": "2024-01-15T10:00:00Z"
                    },
                    {
                        "url": "no-scheme",
                        "title": "skip me",
                        "description": ""
                    }
                ]
            }
        }"#;
        let resp: BraveResponse = serde_json::from_str(body).unwrap();
        let hits = resp.into_hits();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].source, "rust-lang.org");
        assert!(hits[0].published.is_some());
    }
}
