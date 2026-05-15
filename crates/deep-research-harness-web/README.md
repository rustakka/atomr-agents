# atomr-agents-deep-research-harness-web

Axum companion server for `atomr-agents-deep-research-harness`. Exposes a
small JSON API and an SSE event stream, and (with the default
`embed-ui` feature) serves an embedded single-page dashboard for kicking
off and watching research runs.

## Routes

| Method  | Path                            | Description                              |
|---------|---------------------------------|------------------------------------------|
| GET     | `/healthz`                      | Liveness probe.                          |
| GET     | `/api/research`                 | List `ResearchSummary` rows.             |
| POST    | `/api/research`                 | Start a run. Body = `RunRequestBody`.    |
| GET     | `/api/research/:id`             | Full `ResearchResult` snapshot.          |
| DELETE  | `/api/research/:id`             | Delete one result.                       |
| POST    | `/api/research/:id/stop`        | Cooperative cancel.                      |
| GET     | `/api/research/:id/events`      | SSE stream of `DeepResearchEvent`.       |
| GET     | `/api/strategies`               | List available strategy ids.             |
| `/`     | (fallback)                      | SPA HTML / static asset.                 |
