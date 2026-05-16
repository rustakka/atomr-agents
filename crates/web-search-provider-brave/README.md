# atomr-agents-web-search-provider-brave

Brave Search API provider for atomr-agents. Implements
[`atomr_agents_web_search_core::WebSearch`] against
`https://api.search.brave.com/res/v1/web/search` using GET with the
`X-Subscription-Token` header for authentication.

```rust,no_run
use std::sync::Arc;
use atomr_agents_web_search_core::{WebSearch, WebSearchRequest};
use atomr_agents_web_search_provider_brave::{BraveConfig, BraveWebSearch};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let provider = BraveWebSearch::new(BraveConfig::from_env())?;
let hits = provider
    .search(&WebSearchRequest::new("rust async ecosystem").with_max_results(5))
    .await?;
println!("got {} hits", hits.len());
# Ok(()) }
```

Environment variable: `BRAVE_API_KEY`.

Docs: <https://api.search.brave.com/app/documentation/web-search/get-started>.
