# atomr-agents-coding-cli-harness

Harness that wraps local AI coding CLIs as atomr-agents `Callable`s.

```text
                ┌────────────────────────────────────────┐
                │ CodingCliHarness                       │
                │                                        │
   CliRequest ─▶│  VendorRegistry → CliVendor adapter   │
                │  Isolator (Local or Docker)            │
                │  broadcast<CodingCliEvent>             │
                │  HashMap<SessionId, InteractiveSess>   │
                └────────────────────────────────────────┘
                          │                  │
                          ▼                  ▼
                  Headless CliResult   tmux PTY bytes
                                       → broadcast::Sender<Vec<u8>>
```

## Two modes

| Mode         | Surface                                  |
| ------------ | ---------------------------------------- |
| Headless     | `harness.run(req) -> CliResult` + SSE    |
| Interactive  | `harness.start_interactive(req) -> Id` + WS PTY bridge |

## Concept projection

Before every run, the active `CliVendor` materializes the request's
`ConceptProjection` (atomr `Skill` / `Persona` / `Policy` / MCP) into
on-disk CLI config files (`CLAUDE.md`, `AGENTS.md`, `.mcp.json`, etc.).
One-way; atomr is source of truth.

## Callable

`CodingCliHarness` implements `atomr-agents-callable::Callable`, so a
workflow step or another harness can invoke a coding CLI as if it were
just another sub-agent.
