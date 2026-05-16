# atomr-agents-web-search-provider-tavily

Tavily Search API provider for atomr-agents. Implements
[`atomr_agents_web_search_core::WebSearch`] against
`https://api.tavily.com/search` using POST + JSON body authentication.
Tavily returns cleaned-text extracts alongside the snippet, which the
provider surfaces via `WebSearchHit.content` so downstream readers can
skip a fetch-and-extract step.

```rust,no_run
use std::sync::Arc;
use atomr_agents_web_search_core::{WebSearch, WebSearchRequest};
use atomr_agents_web_search_provider_tavily::{TavilyConfig, TavilyWebSearch};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let provider = TavilyWebSearch::new(TavilyConfig::from_env())?;
let hits = provider
    .search(&WebSearchRequest::new("compare actor frameworks in rust"))
    .await?;
println!("got {} hits", hits.len());
# Ok(()) }
```

Environment variable: `TAVILY_API_KEY`.

Docs: <https://docs.tavily.com/docs/rest-api/api-reference>.
