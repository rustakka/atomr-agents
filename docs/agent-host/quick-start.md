# Quick start

Install the host extras (PyYAML is the only new runtime dependency):

```bash
maturin develop
pip install -e '.[host]'
```

Scaffold a host root + a seed agent:

```bash
atomr-host init
# initialized host root at /home/you/.atomr/host
#   + seeded agent at /home/you/.atomr/host/agents/default
```

The seed agent is now on disk. List and inspect it:

```bash
atomr-host agent list
# default

atomr-host agent show default
# agent: default
#   dir:   /home/you/.atomr/host/agents/default
#   model: gpt-4o
#   spec:
#     max_iterations:  8
#     token_budget:    8000
#     ...
```

Edit the persona / rules / memory:

```bash
$EDITOR ~/.atomr/host/agents/default/SOUL.md
$EDITOR ~/.atomr/host/agents/default/RULES.md
$EDITOR ~/.atomr/host/agents/default/MEMORY.md
```

`atomr-host agent show default` re-reads the directory on every
invocation — no daemon to restart.

## Custom root

Override the host root for ephemeral or per-project setups:

```bash
atomr-host --root /tmp/proj-host init
ATOMR_HOST_ROOT=/tmp/proj-host atomr-host agent list
```

Both forms are equivalent; CLI flag wins when both are set.

## Programmatic API

```python
from atomr_agents.agent_host import AgentLoader, HostConfig

cfg = HostConfig.load_default()
loader = AgentLoader(cfg)

# Pure parse — no PyO3 extension required.
defn = loader.parse("default")
print(defn.model, defn.token_budget, [s.id for s in defn.skills])

# Materialize into native AgentSpec / SkillSet / PersonaValue
# (requires `maturin develop`).
loaded = loader.load("default")
print(loaded.spec.id, len(loaded.skill_set), loaded.persona)
```

The pure-parse path is what makes the loader easy to unit-test and
makes the CLI usable in environments where the Rust extension isn't
built yet.

## Creating another agent

```bash
atomr-host agent new researcher --model gpt-4o
$EDITOR ~/.atomr/host/agents/researcher/SOUL.md
atomr-host agent show researcher
```

To remove one:

```bash
atomr-host agent rm researcher       # prompts for confirmation
atomr-host agent rm researcher -f    # no prompt
```

## Full surface (all milestones)

```bash
# M1 — Skeleton + loader (init, agent show/list/rm)
atomr-host init
atomr-host agent show default

# M2 — Local CLI channel
atomr-host chat default

# M3 — Memory + rules
atomr-host memory show default
atomr-host memory sync default
atomr-host rules show default

# M4 — Skills
atomr-host skill new default summarize --keywords tldr,summarize
atomr-host skill ls default
atomr-host skill validate default

# M5 — Hooks
atomr-host hooks ls default
atomr-host hooks test default on_tool_call --payload '{"tool":"shell.exec","text":"api_key=sk-..."}'

# M6 — Crons
atomr-host cron add daily --when 'every:1d' --call '{"kind":"builtin","id":"noop"}'
atomr-host cron ls

# M7 — Routes / multi-channel
atomr-host routes

# M8 — MCP tools
atomr-host mcp add fs --command 'npx @mcp/fs .'
atomr-host mcp ls

# M9 — Curator + events
atomr-host events tail --no-follow
atomr-host skill history default summarize
atomr-host skill revert default summarize
atomr-host skill review default summarize --approve

# M10 — Branching
atomr-host branch ls default
atomr-host branch new default experiment --from main
atomr-host branch diff default main experiment

# M11 — Registry
atomr-host registry ls --kind skill
atomr-host registry resolve skill:summarize@0.1.0

# M12 — Evals
atomr-host eval new smoke
atomr-host eval run default smoke
```

Each milestone has its own page under `docs/agent-host/`: see
[`index.md`](index.md) for the map.
