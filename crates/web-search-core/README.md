# atomr-agents-web-search-core

Provider-agnostic web-search trait + canonical request/hit data types for
the atomr-agents framework.

This crate is intentionally small: it exists so that every search-aware
tool, agent, workflow, or harness in the workspace can depend on a single
trait and a single result shape. Concrete provider integrations
(Tavily, SerpAPI, DuckDuckGo, Brave, …) ship as separate crates and only
need to implement the `WebSearch` trait here.

A deterministic `MockWebSearch` is included so tests and the
`atomr-agents-deep-research-harness` integration tests run end-to-end
without network access.
