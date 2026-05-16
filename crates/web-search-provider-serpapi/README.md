# atomr-agents-web-search-provider-serpapi

SerpAPI (Google) provider for atomr-agents. Implements
[`atomr_agents_web_search_core::WebSearch`] against
`https://serpapi.com/search` using GET + query-string authentication
(`engine=google`). Recency requests are translated to Google's
`tbs=qdr:d|w|m|y` knob.

```rust,no_run
use std::sync::Arc;
use atomr_agents_web_search_core::{WebSearch, WebSearchRequest};
use atomr_agents_web_search_provider_serpapi::{SerpApiConfig, SerpApiWebSearch};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let provider = SerpApiWebSearch::new(SerpApiConfig::from_env())?;
let hits = provider
    .search(&WebSearchRequest::new("rust ecosystem 2024").with_max_results(5))
    .await?;
println!("got {} hits", hits.len());
# Ok(()) }
```

Environment variable: `SERPAPI_KEY`.

Docs: <https://serpapi.com/search-api>.
