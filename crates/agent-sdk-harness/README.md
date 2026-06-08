# atomr-agents-agent-sdk-harness

Harness wrapping Anthropic's **Claude Agent SDK** (the programmable Claude
Code agent) as an atomr-agents `Callable`. Headless one-shot runs and stateful
interactive sessions, normalized event streaming, a `.claude/` projection for
slash commands / skills / MCP, and Anthropic-credit spend tracking via the
shared `SpendLedger`.

The actual SDK is driven behind the pluggable `AgentSdkBackend` trait
(`atomr-agents-agent-sdk-core`). A `MockBackend` keeps Rust tests network-free;
the production `PythonAgentSdkBackend` lives in `py-bindings`.

> **Safety:** the default `permission_mode` is `bypassPermissions` (max
> autonomy — every tool auto-approved). Override per request/spec; run
> untrusted work inside the sandbox harness.

Enable the `actor` feature to expose an interactive session as an
`atomr_core::actor::Actor`. See `docs/agent-sdk-harness.md`.
