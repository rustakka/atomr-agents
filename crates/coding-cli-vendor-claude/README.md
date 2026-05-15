# atomr-agents-coding-cli-vendor-claude

Claude Code adapter for the coding-cli harness.

## Headless invocation

```text
claude -p <prompt>
  --output-format stream-json
  --verbose
  --include-partial-messages
  [--model <id>]
  [--allowed-tools <comma-list>]
  [--resume <session-id>]
```

Emits NDJSON events: `system/init`, `stream_event` with text deltas,
`tool_use`, `tool_result`, `system/api_retry`, and a final `result`
envelope. The parser maps each to a `CodingCliEvent`.

## Concept projection

Before each run the adapter writes:

| atomr concept                                  | File materialized in `workdir`                                |
| ---------------------------------------------- | ------------------------------------------------------------- |
| `PersonaSnapshot` + `project_memory`           | `CLAUDE.md`                                                   |
| each `SkillSnapshot`                           | `.claude/skills/<id>/SKILL.md`                                |
| `ToolSetSnapshot.mcp_servers`                  | `.mcp.json`                                                   |
| `PolicySnapshot.allowed_tools` / `allowed_models` | `.claude/settings.local.json` (permissions)               |
