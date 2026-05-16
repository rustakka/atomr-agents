//! `WebSearch` impl backed by Brave's `GET /res/v1/web/search`.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_web_search_core::{Result, WebSearch, WebSearchError, WebSearchHit, WebSearchRequest};
use reqwest::{header, Client};
use secrecy::ExposeSecret;

use crate::caps::{CAPS, PROVIDER_NAME};
use crate::config::BraveConfig;
use crate::http::{build_http_client, classify_status, retry};
use crate::wire::{build_query, BraveResponse};

pub struct BraveWebSearch {
    config: BraveConfig,
    client: Client,
}

impl BraveWebSearch {
    pub fn new(config: BraveConfig) -> Result<Self> {
        let client = build_http_client(&config.timeouts)?;
        Ok(Self { config, client })
    }

    pub fn with_client(config: BraveConfig, client: Client) -> Self {
        Self { config, client }
    }

    pub fn endpoint(&self) -> &url::Url {
        &self.config.endpoint
    }
}

#[async_trait]
impl WebSearch for BraveWebSearch {
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
        let cap = CAPS.max_results;
        let default_country = self.config.default_country.clone();
        let url = self.config.endpoint.clone();
        let client = self.client.clone();
        let policy = self.config.retry.clone();

        retry(&policy, move || {
            let client = client.clone();
            let url = url.clone();
            let api_key = api_key.clone();
            let default_country = default_country.clone();
            async move {
                let q = build_query(req, cap, default_country.as_deref());
                let resp = client
                    .get(url.clone())
                    .header(header::ACCEPT, "application/json")
                    .header("X-Subscription-Token", api_key.as_str())
                    .query(&q)
                    .send()
                    .await
                    .map_err(|e| WebSearchError::Transport(format!("brave GET: {e}")))?;
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
                let parsed: BraveResponse = resp
                    .json()
                    .await
                    .map_err(|e| WebSearchError::Provider(format!("brave parse: {e}")))?;
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

    fn cfg_for(server: &MockServer) -> BraveConfig {
        let mut cfg = BraveConfig::from_env();
        cfg.endpoint = url::Url::parse(&format!("{}/res/v1/web/search", server.uri())).unwrap();
        cfg.api_key = SecretRef::literal("k-test");
        cfg
    }

    #[tokio::test]
    async fn provider_name_is_stable() {
        let server = MockServer::start().await;
        let provider = BraveWebSearch::new(cfg_for(&server)).unwrap();
        assert_eq!(provider.provider_name(), "brave");
    }

    #[tokio::test]
    async fn search_sends_subscription_token_header() {
        let server = MockServer::start().await;
        let response_body: Value = serde_json::json!({
            "web": {
                "results": [
                    {
                        "url": "https://example.com/a",
                        "title": "Example",
                        "description": "ex",
                        "page_age": "2024-01-02T00:00:00Z"
                    }
                ]
            }
        });
        Mock::given(method("GET"))
            .and(path("/res/v1/web/search"))
            .and(header("X-Subscription-Token", "k-test"))
            .and(query_param("q", "rust ecosystem"))
            .and(query_param("count", "5"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
            .mount(&server)
            .await;

        let provider = BraveWebSearch::new(cfg_for(&server)).unwrap();
        let req = WebSearchRequest::new("rust ecosystem").with_max_results(5);
        let hits = provider.search(&req).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].source, "example.com");
    }

    #[tokio::test]
    async fn freshness_param_is_forwarded() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/res/v1/web/search"))
            .and(query_param("freshness", "pd"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "web": { "results": [] }
            })))
            .mount(&server)
            .await;
        let provider = BraveWebSearch::new(cfg_for(&server)).unwrap();
        let req = WebSearchRequest::new("x").with_recency_days(1);
        let hits = provider.search(&req).await.unwrap();
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn forbidden_surfaces_config_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/res/v1/web/search"))
            .respond_with(ResponseTemplate::new(403).set_body_string("no token"))
            .mount(&server)
            .await;
        let provider = BraveWebSearch::new(cfg_for(&server)).unwrap();
        let err = provider.search(&WebSearchRequest::new("rust")).await.unwrap_err();
        assert!(matches!(err, WebSearchError::Config(_)));
    }

    #[cfg(feature = "integration")]
    #[tokio::test]
    async fn integration_live_search() {
        let key = match std::env::var("BRAVE_API_KEY") {
            Ok(k) if !k.is_empty() => k,
            _ => return,
        };
        let mut cfg = BraveConfig::from_env();
        cfg.api_key = SecretRef::literal(key);
        let provider = BraveWebSearch::new(cfg).unwrap();
        let req = WebSearchRequest::new("rust programming language").with_max_results(3);
        let hits = provider.search(&req).await.unwrap();
        assert!(!hits.is_empty(), "expected at least one hit from brave");
    }
}
