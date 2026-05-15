# atomr-agents-web-search-tool

`WebSearchTool` exposes any `atomr_agents_web_search_core::WebSearch`
provider as an `atomr_agents_tool::Tool`, so agents and harnesses can
call `web_search` uniformly without coupling to a particular provider
implementation.

The tool descriptor advertises a JSON schema matching `WebSearchRequest`
and returns `{ "hits": [WebSearchHit, ...] }`.
