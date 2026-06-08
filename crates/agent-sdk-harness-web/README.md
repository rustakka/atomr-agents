# atomr-agents-agent-sdk-harness-web

Axum REST + SSE web companion for the Claude Agent SDK harness. Start headless
runs and interactive sessions, post messages, interrupt, change permission
mode / model, and stream normalized agent events over SSE.

`cargo run -p atomr-agents-agent-sdk-harness-web --bin server` starts a
dev/demo server backed by the in-memory `MockBackend` (no `claude-agent-sdk`
needed). Production deployments build the harness with the Python-driven
backend and call `WebServer::serve`.

See `docs/agent-sdk-harness.md`.
