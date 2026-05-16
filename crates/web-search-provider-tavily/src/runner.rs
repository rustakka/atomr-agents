//! `WebSearch` impl backed by Tavily's `POST /search`.

use async_trait::async_trait;
use atomr_agents_web_search_core::{Result, WebSearch, WebSearchError, WebSearchHit, WebSearchRequest};
use reqwest::{header, Client};
use secrecy::ExposeSecret;
use std::sync::Arc;

use crate::caps::{CAPS, PROVIDER_NAME};
use crate::config::TavilyConfig;
use crate::http::{build_http_client, classify_status, retry};
use crate::wire::{TavilyRequest, TavilyResponse};

/// Tavily Search provider.
pub struct TavilyWebSearch {
    config: TavilyConfig,
    client: Client,
}

impl TavilyWebSearch {
    /// Build a new provider. Fails only if the underlying HTTP client
    /// can't be constructed (TLS misconfig, etc).
    pub fn new(config: TavilyConfig) -> Result<Self> {
        let client = build_http_client(&config.timeouts)?;
        Ok(Self { config, client })
    }

    /// Build a provider with an injected `reqwest::Client` — useful for
    /// tests that need to point at a `wiremock::MockServer` while still
    /// exercising the real request/response shape.
    pub fn with_client(config: TavilyConfig, client: Client) -> Self {
        Self { config, client }
    }

    /// The provider's configured endpoint. Exposed for telemetry.
    pub fn endpoint(&self) -> &url::Url {
        &self.config.endpoint
    }
}

#[async_trait]
impl WebSearch for TavilyWebSearch {
    async fn search(&self, req: &WebSearchRequest) -> Result<Vec<WebSearchHit>> {
        if req.query.trim().is_empty() {
            return Err(WebSearchError::InvalidRequest("empty query".into()));
        }
        let secret = self
            .config
            .api_key
            .resolve()
            .map_err(|_| WebSearchError::Config("missing or unreadable api key".into()))?;
        // We resolve once and stash the plaintext on the heap behind an
        // `Arc<String>` so the retry closure can re-use it without
        // re-resolving. The string is not logged or serialised beyond
        // the outgoing JSON body. This is the same posture
        // `stt-runtime-deepgram` takes with its `auth_header` string.
        let api_key: Arc<String> = Arc::new(secret.expose_secret().to_string());
        let depth = self.config.default_search_depth.clone();
        let include_answer = self.config.include_answer;
        let cap = CAPS.max_results;
        let url = self.config.endpoint.clone();
        let client = self.client.clone();
        let policy = self.config.retry.clone();

        retry(&policy, move || {
            let client = client.clone();
            let url = url.clone();
            let depth = depth.clone();
            let api_key = api_key.clone();
            async move {
                let body = TavilyRequest::build(api_key.as_str(), &depth, include_answer, req, cap);
                let resp = client
                    .post(url.clone())
                    .header(header::ACCEPT, "application/json")
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| WebSearchError::Transport(format!("tavily POST: {e}")))?;

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
                let parsed: TavilyResponse = resp
                    .json()
                    .await
                    .map_err(|e| WebSearchError::Provider(format!("tavily parse: {e}")))?;
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
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn cfg_for(server: &MockServer) -> TavilyConfig {
        let mut cfg = TavilyConfig::from_env();
        cfg.endpoint = url::Url::parse(&format!("{}/search", server.uri())).unwrap();
        cfg.api_key = SecretRef::literal("k-test");
        cfg
    }

    #[tokio::test]
    async fn provider_name_is_stable() {
        let server = MockServer::start().await;
        let provider = TavilyWebSearch::new(cfg_for(&server)).unwrap();
        assert_eq!(provider.provider_name(), "tavily");
    }

    #[tokio::test]
    async fn search_posts_api_key_in_body_and_parses_results() {
        let server = MockServer::start().await;
        let response_body: Value = serde_json::json!({
            "answer": "synth",
            "results": [
                {
                    "url": "https://rust-lang.org/",
                    "title": "Rust",
                    "content": "Rust is a language",
                    "score": 0.9,
                    "published_date": "2024-01-02"
                }
            ]
        });
        Mock::given(method("POST"))
            .and(path("/search"))
            .and(header("accept", "application/json"))
            .and(body_partial_json(serde_json::json!({
                "api_key": "k-test",
                "query": "rust",
                "search_depth": "basic"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
            .mount(&server)
            .await;

        let provider = TavilyWebSearch::new(cfg_for(&server)).unwrap();
        let req = WebSearchRequest::new("rust").with_max_results(3);
        let hits = provider.search(&req).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Rust");
        assert_eq!(hits[0].source, "rust-lang.org");
        assert_eq!(hits[0].content.as_deref(), Some("Rust is a language"));
    }

    #[tokio::test]
    async fn empty_query_is_rejected() {
        let server = MockServer::start().await;
        let provider = TavilyWebSearch::new(cfg_for(&server)).unwrap();
        let err = provider.search(&WebSearchRequest::new("   ")).await.unwrap_err();
        assert!(matches!(err, WebSearchError::InvalidRequest(_)));
    }

    #[tokio::test]
    async fn unauthorized_surfaces_config_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/search"))
            .respond_with(ResponseTemplate::new(401).set_body_string("nope"))
            .mount(&server)
            .await;
        let provider = TavilyWebSearch::new(cfg_for(&server)).unwrap();
        let err = provider.search(&WebSearchRequest::new("rust")).await.unwrap_err();
        assert!(matches!(err, WebSearchError::Config(_)));
    }

    // ---- Live integration test, gated on TAVILY_API_KEY. ----
    #[cfg(feature = "integration")]
    #[tokio::test]
    async fn integration_live_search() {
        let key = match std::env::var("TAVILY_API_KEY") {
            Ok(k) if !k.is_empty() => k,
            _ => return,
        };
        let mut cfg = TavilyConfig::from_env();
        cfg.api_key = SecretRef::literal(key);
        let provider = TavilyWebSearch::new(cfg).unwrap();
        let req = WebSearchRequest::new("rustlang programming language").with_max_results(3);
        let hits = provider.search(&req).await.unwrap();
        assert!(!hits.is_empty(), "expected at least one hit from tavily");
    }
}
