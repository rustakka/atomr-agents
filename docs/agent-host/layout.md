# On-disk layout

The host's source of truth is a directory tree under
**`~/.atomr/host/`** (override with `$ATOMR_HOST_ROOT`). Markdown files
are human-editable and authoritative; SQLite/vector indexes under
`state/` are derived and rebuildable.

```
~/.atomr/host/
├── config.yaml                 # providers, default model, root settings
├── AGENTS.md                   # multi-agent routing rules (consumed in M7)
├── agents/
│   └── <agent-id>/
│       ├── agent.yaml          # the spec — id, model, budgets, references
│       ├── SOUL.md             # persona (≤500 words; YAML frontmatter)
│       ├── RULES.md            # behavioral rules (one per bullet)
│       ├── MEMORY.md           # auto-curated facts (one per bullet)
│       ├── USER.md             # user profile (one per bullet)
│       ├── skills/
│       │   └── <skill-id>/SKILL.md   # YAML frontmatter + body
│       ├── hooks/
│       │   └── <event>.yaml          # on_turn_start, on_tool_call, on_error
│       └── state/
│           ├── memory.db                 # SQLite + FTS5 (derived; M3)
│           ├── vectors/                  # embedding index (optional; M3)
│           ├── threads/<channel-id>/<thread-id>.jsonl   # append-only turn log
│           └── checkpoints/<branch-id>/<ts>.json        # branch snapshots (M10)
├── channels/
│   └── <channel-id>.yaml       # ChannelHarness provider config (M2 / M7)
├── crons/
│   └── <cron-id>.yaml          # cron entry → target callable (M6)
├── tools/
│   └── <tool-id>.yaml          # tool descriptor or MCP server (M8)
├── registry/                   # cached published Skill / Persona / Tool bundles (M11)
└── events.jsonl                # rolling EventBus log (M9)
```

## `config.yaml`

```yaml
version: 1
default_agent: default
default_model: gpt-4o
providers:
  openai:
    kind: openai
    api_key_env: OPENAI_API_KEY
  anthropic:
    kind: anthropic
    api_key_env: ANTHROPIC_API_KEY
```

`api_key_env` is the *name* of an environment variable. No secret
value is ever written to disk.

## `agents/<id>/agent.yaml`

```yaml
id: alpha
model: gpt-4o
max_iterations: 6
token_budget: 4096
time_budget_ms: 45000
money_budget_usd: 0.50
skillset_id: alpha-skills
skillset_version: 0.2.0
curation:
  strategy: auto_promote          # or: human_approval
  params:
    min_success_rate: 0.8
    history_limit: 20
```

Defaults match `AgentSpec::default_budgets()` in
`crates/agent/src/spec.rs`. The `curation` block is read in M9.

## `agents/<id>/SOUL.md`

```markdown
---
identity: A pragmatic engineering pair-programmer.
style:
  tone: dry
  register: technical
  verbosity: 1
traits:
  - label: rigorous
    weight: 0.9
    description: Reads the failing test before guessing.
metadata:
  framework: atomr-agents
---

# Soul

Long-form persona prose lives in the body. It is rendered into the
instruction prefix on every turn, budget permitting.
```

Maps to `PersonaValue` / `TraitFragment` / `StyleSpec` / `PersonaMetadata`.

## `agents/<id>/RULES.md`

Each non-empty bullet (`- ...`) becomes one instruction fragment.
Lines starting with `#` are treated as headings and dropped. Identical
parser is used for `MEMORY.md` and `USER.md`.

```markdown
# Rules

- Always read the failing test before suggesting a fix.
- Surface tool failures rather than retrying silently.
- Prefer one clarifying question over guessing.
```

## `agents/<id>/skills/<skill-id>/SKILL.md`

```markdown
---
name: Summarize
priority: 7
keywords:
  - summarize
  - tldr
tool_overlay:
  - text.summarize
memory_namespace:
  - alpha
  - skill
  - summarize
---

When the user asks for a summary, produce a 3-bullet TL;DR that
captures the load-bearing facts and the open questions.
```

Maps to `Skill` (`crates/skill/src/lib.rs`). The body becomes
`instruction_fragment`.

## `agents/<id>/hooks/<event>.yaml`

```yaml
event: on_tool_call
match:
  tool: shell.exec
call:
  kind: skill
  id: redact_secrets
when: pre               # pre | post | both
budget:
  tokens: 2000
  ms: 5000
```

Hooks are parsed in M1 and dispatched in M5.

## Invariants

- **Markdown files are human-editable.** A file watcher will reindex
  the derived SQLite/vector stores on save (M3).
- **`agent.yaml` is the single source for `AgentSpec` assembly.**
  Strategies are referenced by name; parameters live inline.
- **Threads are owned by `ChannelHarness`** (`crates/channel-harness`).
  The host only persists their state under `state/threads/`.
- **Markdown is authoritative.** Anything under `state/` can be deleted
  and rebuilt from the Markdown files.
