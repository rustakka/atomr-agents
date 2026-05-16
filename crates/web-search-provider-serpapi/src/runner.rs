//! `WebSearch` impl backed by SerpAPI's `GET /search`.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_web_search_core::{Result, WebSearch, WebSearchError, WebSearchHit, WebSearchRequest};
use reqwest::{header, Client};
use secrecy::ExposeSecret;

use crate::caps::{CAPS, PROVIDER_NAME};
use crate::config::SerpApiConfig;
use crate::http::{build_http_client, classify_status, retry};
use crate::wire::{build_query, SerpApiResponse};

pub struct SerpApiWebSearch {
    config: SerpApiConfig,
    client: Client,
}

impl SerpApiWebSearch {
    pub fn new(config: SerpApiConfig) -> Result<Self> {
        let client = build_http_client(&config.timeouts)?;
        Ok(Self { config, client })
    }

    pub fn with_client(config: SerpApiConfig, client: Client) -> Self {
        Self { config, client }
    }

    pub fn endpoint(&self) -> &url::Url {
        &self.config.endpoint
    }
}

#[async_trait]
impl WebSearch for SerpApiWebSearch {
    async fn search(&self, req: &WebSearchRequest) -> Result<Vec<WebSearchHit>> {
        if req.query.trim().is_empty() {
            return Err(WebSearchError::InvalidRequest("empty query".into()));
        }
        let secret = self
            .config
            .api_key
            .resolve()
            .map_err(|_| WebSearchError::Config("missing or unreadable api key".into()))?;
        let api_key: Arc<String> = Arc::new(secret.expose_secret().to_string());
        let engine = self.config.engine.clone();
        let cap = CAPS.max_results;
        let url = self.config.endpoint.clone();
        let client = self.client.clone();
        let policy = self.config.retry.clone();

        retry(&policy, move || {
            let client = client.clone();
            let url = url.clone();
            let engine = engine.clone();
            let api_key = api_key.clone();
            async move {
                let q = build_query(api_key.as_str(), &engine, req, cap);
                let resp = client
                    .get(url.clone())
                    .header(header::ACCEPT, "application/json")
                    .query(&q)
                    .send()
                    .await
                    .map_err(|e| WebSearchError::Transport(format!("serpapi GET: {e}")))?;
                let status = resp.status().as_u16();
                if !resp.status().is_success() {
                    let retry_after = resp
                        .headers()
                        .get(header::RETRY_AFTER)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok());
                    let text = resp.text().await.unwrap_or_default();
                    return Err(classify_status(status, retry_after, text));
                }
                let parsed: SerpApiResponse = resp
                    .json()
                    .await
                    .map_err(|e| WebSearchError::Provider(format!("serpapi parse: {e}")))?;
                Ok(parsed.into_hits())
            }
        })
        .await
    }

    fn provider_name(&self) -> &str {
        PROVIDER_NAME
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_stt_remote_core::SecretRef;
    use serde_json::Value;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn cfg_for(server: &MockServer) -> SerpApiConfig {
        let mut cfg = SerpApiConfig::from_env();
        cfg.endpoint = url::Url::parse(&format!("{}/search", server.uri())).unwrap();
        cfg.api_key = SecretRef::literal("k-test");
        cfg
    }

    #[tokio::test]
    async fn provider_name_is_stable() {
        let server = MockServer::start().await;
        let provider = SerpApiWebSearch::new(cfg_for(&server)).unwrap();
        assert_eq!(provider.provider_name(), "serpapi");
    }

    #[tokio::test]
    async fn search_sends_api_key_in_query_string() {
        let server = MockServer::start().await;
        let response_body: Value = serde_json::json!({
            "search_metadata": { "status": "Success" },
            "organic_results": [
                {
                    "link": "https://example.com/a",
                    "title": "Example",
                    "snippet": "ex",
                    "date": "Jan 15, 2024",
                    "position": 1
                }
            ]
        });
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("api_key", "k-test"))
            .and(query_param("engine", "google"))
            .and(query_param("q", "rust ecosystem"))
            .and(query_param("num", "5"))
            .and(header("accept", "application/json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
            .mount(&server)
            .await;

        let provider = SerpApiWebSearch::new(cfg_for(&server)).unwrap();
        let req = WebSearchRequest::new("rust ecosystem").with_max_results(5);
        let hits = provider.search(&req).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Example");
        assert_eq!(hits[0].source, "example.com");
    }

    #[tokio::test]
    async fn recency_days_set_tbs_qdr() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("tbs", "qdr:m"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "organic_results": []
            })))
            .mount(&server)
            .await;
        let provider = SerpApiWebSearch::new(cfg_for(&server)).unwrap();
        let req = WebSearchRequest::new("x").with_recency_days(14);
        let hits = provider.search(&req).await.unwrap();
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn server_error_surfaces_provider_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal"))
            .mount(&server)
            .await;
        let mut cfg = cfg_for(&server);
        // Disable retries so the test runs fast.
        cfg.retry.max_attempts = 1;
        let provider = SerpApiWebSearch::new(cfg).unwrap();
        let err = provider.search(&WebSearchRequest::new("rust")).await.unwrap_err();
        assert!(matches!(err, WebSearchError::Provider(_)));
    }

    #[cfg(feature = "integration")]
    #[tokio::test]
    async fn integration_live_search() {
        let key = match std::env::var("SERPAPI_KEY") {
            Ok(k) if !k.is_empty() => k,
            _ => return,
        };
        let mut cfg = SerpApiConfig::from_env();
        cfg.api_key = SecretRef::literal(key);
        let provider = SerpApiWebSearch::new(cfg).unwrap();
        let req = WebSearchRequest::new("rust programming language").with_max_results(3);
        let hits = provider.search(&req).await.unwrap();
        assert!(!hits.is_empty(), "expected at least one hit from serpapi");
    }
}
