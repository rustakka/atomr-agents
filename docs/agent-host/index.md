# atomr-agents-host

A long-lived process + on-disk layout that gives an atomr-agents agent
**persistent identity, memory, skills, rules, tools, hooks, schedules,
and inbound channels** — the same role Claude Code plays for the
Claude model.

## Why a host?

atomr-agents ships the *primitives* — `AgentSpec`, `Persona`, `Skill`,
`Tool`, `MemoryStore`, `ChannelHarness`, eval, observability. To run an
agent across days/months/users you need a place to *put* those
primitives, watch them for edits, and orchestrate their long-lived
behavior. That place is the host.

The host borrows three load-bearing ideas from prior art:

| Idea | Borrowed from |
|------|--------------|
| File-based human-readable SOUL/MEMORY/USER/RULES Markdown | OpenClaw, Hermes Agent |
| Multi-channel gateway (one agent across CLI/WhatsApp/Discord/...) | OpenClaw gateway, Hermes' 20+ channels |
| Auto-curated skills + cron / heartbeat scheduling | Hermes Agent |

…and maps every borrowed concept onto an *existing* atomr-agents
primitive. The host does not introduce a parallel agent runtime —
it composes the one that already ships.

## Implementation status

| Milestone | Status | Surface |
|-----------|--------|---------|
| M1 — Skeleton + AgentLoader | ✅ implemented | `atomr_agents.agent_host` + `atomr-host` CLI |
| M2 — Local CLI channel + AgentRouter | ✅ implemented | `atomr-host chat`, [`chat.md`](chat.md) |
| M3 — MarkdownMemorySync + RULES rendering | ✅ implemented | `atomr-host memory/rules`, [`memory.md`](memory.md) |
| M4 — Skills (SKILL.md → Skill → SkillSet) | ✅ implemented | `atomr-host skill new/ls/validate`, [`skills.md`](skills.md) |
| M5 — Hooks (EventBus filter, parallel dispatch) | ✅ implemented | `atomr-host hooks ls/test`, [`hooks.md`](hooks.md) |
| M6 — Scheduler + crons | ✅ implemented | `atomr-host cron add/ls/rm`, [`scheduler.md`](scheduler.md) |
| M7 — Multi-channel gateway (AGENTS.md routing) | ✅ implemented | `atomr-host routes`, [`gateway.md`](gateway.md) |
| M8 — MCP bridge | ✅ implemented (stub transport) | `atomr-host mcp add/ls`, [`mcp.md`](mcp.md) |
| M9 — SkillCurator + CurationStrategy + events tail | ✅ implemented | `atomr-host events/skill review/history/revert`, [`curator.md`](curator.md) |
| M10 — Branching / checkpoints | ✅ implemented | `atomr-host branch ls/new/switch/diff/rm`, [`branching.md`](branching.md) |
| M11 — Registry pull | ✅ implemented | `atomr-host registry ls/resolve`, [`registry.md`](registry.md) |
| M12 — Eval harness wiring | ✅ implemented | `atomr-host eval new/ls/run`, [`evals.md`](evals.md) |

## Quick start

```bash
maturin develop
pip install -e '.[host]'              # pulls PyYAML
atomr-host init                       # ~/.atomr/host/ scaffolded with a `default` agent
atomr-host agent list
atomr-host agent show default
```

See [`quick-start.md`](quick-start.md) for the full first-five-minutes
flow and [`layout.md`](layout.md) for the on-disk format.

## Naming note

The host's Python surface lives at **`atomr_agents.agent_host`**, not
`atomr_agents.host`. The pre-existing `atomr_agents.host` module is
the *host-mode* facade (mirrors `atomr-infer`'s `host.py` — Python
code driving Rust strategies); it pre-dates this work and is
unrelated to the long-lived runtime.

## Implementation note — pure-Python

The plan called for a new `crates/host` Rust crate. The shipped M1-M12
implementation is **pure Python**, layered on top of the existing
PyO3-bound native types (`AgentSpec`, `Skill`, `SkillSet`,
`PersonaValue`, `MemoryStore`, `ChannelHarness`, `ChatPromptTemplate`,
`Registry`). The deviation was deliberate: pure-Python lets the
loader, scheduler, hook dispatcher, gateway, curator, branching,
registry cache, and eval runner all evolve at iteration speed without
PyO3 round-trips. A `crates/host` Rust crate becomes valuable once
the substrate stabilizes and we need hot-reload / cron / MCP at
native speed; the public Python surface is shaped so that
introduction stays additive.

## Tests

```bash
PYTHONPATH=python python3.12 -m pytest python/atomr_agents/tests/test_host_*.py -q
# 262 passed, 2 skipped (no-native fallback paths)
PYTHONPATH=python python3.14 -m pytest python/atomr_agents/tests/test_host_*.py -q
# 248 passed, 16 skipped (the 3.14 wheel panics on ChannelHarness init;
# channel- and memory-store-touching tests skip gracefully)
```
