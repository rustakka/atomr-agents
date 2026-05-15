# atomr-agents-coding-cli-vendor-codex

OpenAI Codex CLI adapter for the coding-cli harness.

## Headless invocation

```text
codex exec <prompt>
  [--model <id>]
  [--full-access]    # if PolicySnapshot.auto_approve_unrestricted
```

Codex's stdout schema is less standardized than Claude's. The parser
emits `AssistantTextDelta` for plain text lines, normalizes any JSON
envelopes it can recognize (`assistant`, `tool_call`, `tool_result`,
`usage`), and falls back to `RawVendorEvent` for anything else.

## Concept projection

| atomr concept                              | File materialized in `workdir`        |
| ------------------------------------------ | ------------------------------------- |
| `PersonaSnapshot` + skills + project memory | `AGENTS.md`                          |
| `ToolSetSnapshot.mcp_servers`              | `.codex/config.toml` `[mcp_servers]` |
