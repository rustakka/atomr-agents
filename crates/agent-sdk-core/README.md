# atomr-agents-agent-sdk-core

Contract layer for the **Claude Agent SDK** harness — config that mirrors the
SDK's `ClaudeAgentOptions`, a normalized message/result/event schema, and the
pluggable `AgentSdkBackend` / `AgentSdkSession` trait seam (with an in-memory
`MockBackend` for network-free tests).

Wraps Anthropic's programmable Claude Code agent
(`claude-agent-sdk` / `@anthropic-ai/claude-agent-sdk`). Distinct from the
server-side "Managed Agents" API.

See `docs/agent-sdk-harness.md` for the full design.
