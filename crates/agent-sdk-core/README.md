# atomr-agents-agent-sdk-core

Contract layer for the **Claude Agent SDK** harness — config that mirrors the
SDK's `ClaudeAgentOptions`, a normalized message/result/event schema, and the
pluggable `AgentSdkBackend` / `AgentSdkSession` trait seam (with an in-memory
`MockBackend` for network-free tests).

Wraps Anthropic's programmable Claude Code agent
(`claude-agent-sdk` / `@anthropic-ai/claude-agent-sdk`). Distinct from the
server-side "Managed Agents" API.

The contract is **provider-neutral**: everything above the `AgentSdkBackend`
trait sees only the normalized schema, so other vendors that mirror
Anthropic's Agent SDK shape (with small option/message differences) plug in via
a thin adapter without touching the harness. See *Generalizing to other
providers* in `docs/agent-sdk-harness.md`.

See `docs/agent-sdk-harness.md` for the full design.
