# Coding CLI harness

Wraps local AI coding CLIs (Claude Code, OpenAI Codex CLI, Google
Gemini CLI) as atomr-agents `Callable`s. Two modes against the same
vendor adapter:

- **Headless** — non-interactive, structured event stream. Use this
  when the harness is *driving* the CLI as a sub-agent.
- **Interactive** — tmux-wrapped TUI, bridged over a PTY to an
  xterm.js terminal in the browser. Use this when a human operator is
  driving the CLI and you want atomr to observe / persist the session.

Both modes broadcast a normalized `CodingCliEvent` stream on the
harness's `broadcast::Sender`.

## Crate layout

```
crates/coding-cli-core/             # shared types: CliRequest, CliResult, CodingCliEvent, CliVendor trait
crates/coding-cli-isolator/         # Isolator trait + LocalIsolator + DockerIsolator (bollard)
crates/coding-cli-vendor-claude/    # claude -p --output-format stream-json + CLAUDE.md / .mcp.json projection
crates/coding-cli-vendor-codex/     # codex exec + AGENTS.md projection
crates/coding-cli-vendor-gemini/    # gemini -p --output-format stream-json + system-instructions projection
crates/coding-cli-harness/          # CodingCliHarness (Callable), broadcast<CodingCliEvent>, session registry
crates/coding-cli-harness-web/      # Axum REST + SSE + WebSocket + embedded SPA on port 7300
```

Python parity via `crates/py-bindings/src/coding_cli.rs` → the
`atomr_agents.coding_cli` facade.

## Lifecycle

```
                  ┌─────────────────────────────────────────────────────┐
   CliRequest ──▶ │ CodingCliHarness                                    │
                  │                                                     │
                  │  1. VendorRegistry.get(req.vendor)                  │
                  │  2. vendor.materialize_config(projection, workdir)  │
                  │      └── writes CLAUDE.md / AGENTS.md / .mcp.json   │
                  │  3. vendor.build_*_command(req, workdir)            │
                  │  4. isolator.spawn(cmd) → ProcessHandle             │
                  │                                                     │
                  │   ┌─ headless ──────────────────────────────────┐   │
                  │   │ vendor.new_parser() drives the NDJSON      │   │
                  │   │ stream into normalized CodingCliEvent;     │   │
                  │   │ events broadcast and accumulate into a     │   │
                  │   │ CliResult; CliRunStore.put(result).        │   │
                  │   └────────────────────────────────────────────┘   │
                  │                                                     │
                  │   ┌─ interactive ───────────────────────────────┐   │
                  │   │ tmux new-session -d → headless spawn        │   │
                  │   │ tmux attach-session → PTY spawn             │   │
                  │   │ pty_pump fans bytes ↔ broadcast<Vec<u8>> /   │   │
                  │   │ mpsc<SessionTransport>; registered in       │   │
                  │   │ SessionRegistry by CliSessionId.            │   │
                  │   └────────────────────────────────────────────┘   │
                  └─────────────────────────────────────────────────────┘
```

## Concept projection

| atomr concept                     | Claude Code               | Codex CLI                 | Gemini CLI                                |
| --------------------------------- | ------------------------- | ------------------------- | ----------------------------------------- |
| `PersonaSnapshot`                 | top of `CLAUDE.md`        | top of `AGENTS.md`        | top of `.gemini/system_instructions.md`   |
| `SkillSnapshot[]`                 | `.claude/skills/<id>/SKILL.md` | inlined in `AGENTS.md` | inlined in `system_instructions.md` |
| `PolicySnapshot.allowed_tools`    | `--allowed-tools` + `settings.local.json` | (instruction text only) | (instruction text only)           |
| `PolicySnapshot.auto_approve_*`   | `--permission-mode acceptEdits` | `--full-access`     | `--yolo`                                   |
| `ToolSetSnapshot.mcp_servers`     | `.mcp.json`               | `.codex/config.toml`      | `.gemini/settings.json`                   |
| `project_memory` (string)         | section of `CLAUDE.md`    | section of `AGENTS.md`    | section of `system_instructions.md`       |

The projection is **one-way**: atomr is the source of truth; the
adapter overwrites the vendor's files before every run.

## Normalized event schema

`CodingCliEvent` is a `serde(tag = "kind", rename_all = "snake_case")`
enum with variants:

| Variant                  | When                                                  |
| ------------------------ | ----------------------------------------------------- |
| `run_started`            | Harness spawned the CLI process                       |
| `system_init`            | Vendor reported its tools / MCP / plugins loaded      |
| `assistant_text_delta`   | Streaming assistant text                              |
| `tool_call_started`      | Vendor began invoking a tool                          |
| `tool_call_finished`     | Vendor's tool call returned                           |
| `api_retry`              | Vendor reported a retryable API error                 |
| `usage`                  | Token / cost accounting                               |
| `run_finished`           | Terminal event (CLI exited)                           |
| `raw_vendor_event`       | Pass-through for events the normalizer didn't map yet |
| `note`                   | Free-form diagnostic (stderr lines, parse warnings)   |

## Isolation

| Backend         | When                                       | Notes                                                     |
| --------------- | ------------------------------------------ | --------------------------------------------------------- |
| `LocalIsolator` | `IsolationSpec::Local`                     | `tokio::process` for headless, `portable-pty` for PTY     |
| `DockerIsolator`| `IsolationSpec::Docker { image, mounts, env, network }` | `bollard` create + attach; workdir bind-mounted to `/workspace` (default) |

Default per-vendor images live under
`crates/coding-cli-isolator/images/`:

- `claude.Dockerfile` (node + `@anthropic-ai/claude-code`)
- `codex.Dockerfile` (node + `@openai/codex`)
- `gemini.Dockerfile` (node + `@google/gemini-cli`)

All three install `tmux` so interactive mode works in-container too.

## Web companion

`atomr-agents-coding-cli-harness-web` exposes an Axum server on port
`7300` (next slot after `7200` deep-research).

| Method | Path                                | Purpose                                |
| ------ | ----------------------------------- | -------------------------------------- |
| GET    | `/api/cli/vendors`                  | Wired-up vendors                       |
| POST   | `/api/cli/runs`                     | Start a headless run                   |
| GET    | `/api/cli/runs`                     | Recent runs                            |
| GET    | `/api/cli/runs/:id`                 | One run                                |
| GET    | `/api/cli/runs/events`              | SSE of normalized events               |
| POST   | `/api/cli/sessions`                 | Start an interactive session           |
| GET    | `/api/cli/sessions`                 | Active interactive sessions            |
| DELETE | `/api/cli/sessions/:id`             | Stop an interactive session            |
| GET    | `/api/cli/sessions/:id/io` *(WS)*   | xterm.js terminal byte bridge          |

The embedded SPA in `ui/` uses xterm.js + addon-fit from a CDN and
binds keystrokes / resizes over the WebSocket.

## Python parity

```python
from atomr_agents.coding_cli import CodingCliHarness

harness = CodingCliHarness.local_default()

# Headless
result = await harness.run_headless({
    "vendor": "claude",
    "mode": "headless",
    "prompt": "list files in src/",
    "workdir": "/path/to/repo",
})
print(result["final_text"])

# Interactive
session = await harness.start_interactive({
    "vendor": "claude",
    "mode": "interactive",
    "workdir": "/path/to/repo",
})
await session.send_keys(b"ls\n")
chunk = await session.read()   # bytes
await session.stop()
```

The harness implements `atomr_agents_callable::Callable`, so a Rust
workflow / harness can invoke a coding CLI as a sub-step the same way
it would invoke any other callable.

## Adding a new vendor

1. Add a `crates/coding-cli-vendor-<name>` crate.
2. Implement `CliVendor` from `coding-cli-core` (command builder, NDJSON parser, mapper).
3. Add a feature flag to `coding-cli-harness/Cargo.toml` and register the adapter in `VendorRegistry::default_vendors()`.
4. Add a Dockerfile to `coding-cli-isolator/images/`.
5. Ship fixture tests for the parser against canned NDJSON.

The three v1 adapters average ~600 LoC each (command + parser + mapper
+ tests). The vendor seam is intentionally narrow.
