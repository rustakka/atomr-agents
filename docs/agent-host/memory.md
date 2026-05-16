# MarkdownMemorySync + RULES rendering (M3)

M3 wires the agent's Markdown files into the runtime:

* **`MEMORY.md`** / **`USER.md`** bullets become items in a native
  `MemoryStore`, retrievable by tag (`memory_md` / `user_md`).
* **`SOUL.md`** + **`RULES.md`** + **`MEMORY.md`** + **`USER.md`**
  compose into a system prompt and a native `ChatPromptTemplate`.

The on-disk Markdown remains authoritative. The store is a derived
index — clearing it and re-running `memory sync` reproduces it.

## Sync facts into a MemoryStore

```python
import asyncio
from atomr_agents import _native
from atomr_agents.agent_host import AgentLoader, HostConfig, sync_all, list_memory_facts

cfg = HostConfig.load_default()
loaded = AgentLoader(cfg).load("default")

store = _native.memory.in_memory_store()
counts = asyncio.run(sync_all(loaded, store))
# {'memory_md': 1, 'user_md': 2}

items = asyncio.run(list_memory_facts(loaded, store))
for it in items:
    print(it.payload["text"])
```

Each fact lands at the agent's `MemoryNamespace.agent(<id>)` with:

| field        | value                                |
|--------------|--------------------------------------|
| `id`         | `memory_md:<i>` or `user_md:<i>`     |
| `kind`       | `MemoryKind.semantic()`              |
| `tags`       | `["memory_md"]` or `["user_md"]`     |
| `payload`    | `{"text": "<bullet>"}`               |

The id is 1-indexed by source order so re-syncing overwrites the
matching item in place (no manual delete dance).

## Reload after on-disk edits

```python
from atomr_agents.agent_host import reload_agent

# Open MEMORY.md in $EDITOR, add a bullet, save.
loaded = reload_agent(cfg, "default")
asyncio.run(sync_all(loaded, store))     # picks up the new bullet
```

A file watcher (M5 / cross-cutting) will eventually call this
automatically; for now it's manual / cron-driven.

## Render the system prompt

```python
from atomr_agents.agent_host import build_system_prompt

print(build_system_prompt(loaded))
# # Persona
#
# A helpful general-purpose assistant.
#
# - helpful (weight 0.9): Prioritize concrete, actionable answers.
# - concise (weight 0.7): Prefer brevity over verbosity; ask before expanding.
#
# # Rules
#
# - Always acknowledge the user's request before acting on it.
# - ...
```

Block order is fixed (Persona → Rules → Memory → About the user).
Empty blocks are dropped — an agent with no rules and no memory still
renders persona-only. If *every* block is empty, the prompt falls
back to `You are <agent_id>.` so the model never sees an empty
system message.

The `user_facts` keyword argument overrides what gets rendered into
the "About the user" block — useful when you want to slot in a
freshly-curated profile rather than the on-disk `USER.md` body.

## Build a ChatPromptTemplate

```python
from atomr_agents.agent_host import build_chat_prompt_template

template = build_chat_prompt_template(loaded)
rendered = template.render({"user_message": "Hi!"})
for m in rendered:
    print(m.role, "=", m.content[:80])
```

The template has two messages: a `system` message holding the
composed Markdown prompt, and a `user` message with a single
placeholder `{user_message}`. Plug it into any `InferenceClient`
that consumes a list of `RenderedMessage`s.

## CLI

```bash
atomr-host memory show default      # parsed MEMORY.md / USER.md bullets
atomr-host memory sync default      # push to an in-memory MemoryStore
atomr-host rules show default       # rendered system prompt
```

`memory sync` instantiates a fresh in-memory store per invocation —
durable storage lands in M10 (branching / checkpoints).

## Native gating

All three sync functions require the PyO3 extension. They raise a
clear `AgentHostError` when `_native` is missing rather than failing
with a vague `AttributeError`.

The pure-Python block renderers (`render_persona_block`,
`render_rules_block`, `render_memory_block`, `render_user_block`,
`build_system_prompt`) work *without* the extension, so they can be
used by tooling that doesn't need to talk to the runtime.

## Known issue — Python 3.14 wheel

The same Tokio-runtime issue that affects M2's `ChannelHarness` also
affects the `MemoryStore.put` async path under the currently-built
3.14 wheel. M3 tests use a subprocess probe (rather than just
`try/except`) to detect this case, and skip cleanly when it
triggers. The fix is tracked alongside the maturin matrix update.
