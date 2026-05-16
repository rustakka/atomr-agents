# Hooks (M5)

Hooks are small callables that fire on a named event with a JSON
payload. The host parses `agents/<id>/hooks/<event>.yaml` files
during M1 and M5 adds the substrate to actually run them — a
registry, a dispatcher that fans out via `asyncio.gather` with
per-hook budgets, and two built-in implementations.

## A hook on disk

```yaml
# agents/default/hooks/on_tool_call.yaml
event: on_tool_call
match:
  tool: shell.exec
call:
  kind: builtin
  id: redact_secrets
when: pre               # pre | post | both
budget:
  ms: 2000              # per-hook timeout in ms
  tokens: 2000          # advisory budget passed in HookResult.ctx (future)
```

- **`event`** — the event name this hook subscribes to.
- **`match`** — every key/value must match `payload[key] == value`. An
  empty `match: {}` always fires.
- **`call`** — what to run. Built-ins right now: `redact_secrets`,
  `record_to_jsonl`. Anything else returns `None` from the default
  resolver so the caller can bind a custom impl.
- **`when`** — `pre` runs before the action, `post` after. M5 ships
  the substrate; the chat / tool pipelines plug into pre/post in M6+.
- **`budget.ms`** — per-hook timeout enforced by `asyncio.wait_for`.

## Programmatic dispatch

```python
import asyncio
from atomr_agents.agent_host import (
    AgentLoader, HostConfig, HookRegistry, HookDispatcher, default_hook_resolver,
)

cfg = HostConfig.load_default()
defn = AgentLoader(cfg).parse("default")

registry = HookRegistry()
resolver = default_hook_resolver(jsonl_path=cfg.paths.events_jsonl)
registry.register_definitions(defn.hooks, resolver)

dispatcher = HookDispatcher(registry)
results = asyncio.run(
    dispatcher.dispatch(
        "on_tool_call",
        {"tool": "shell.exec", "text": "api_key=sk-..."},
        when="pre",
    )
)
for r in results:
    print(r.hook_id, r.ok, r.duration_ms, r.error)
```

`HookResult.output` is whatever the hook returned (typically a
shallow-modified payload dict). Failures — exceptions or timeouts —
become `HookResult(ok=False, error="...")`; `dispatch()` never raises
out of a failed hook.

## CLI

```bash
atomr-host hooks ls default
# on_tool_call  when=pre  match=[tool=shell.exec]  call=redact_secrets

atomr-host hooks test default on_tool_call \
    --payload '{"tool":"shell.exec","text":"api_key=sk-abc..."}'
# dispatched on_tool_call → 1 hook(s)
#   [OK] on_tool_call#0 (0.0ms when=pre) output_keys=['tool', 'text']
```

`hooks test` is the M5 smoke-test surface — useful while authoring a
new hook or before promoting one across agents. Exit code is `0` when
every fired hook returned `ok=True`, `1` otherwise.

## Built-in: `redact_secrets`

Scrubs the named field (default `text`) of a payload against common
secret patterns (`api_key=`, `sk-…`, `AKIA…`, `secret=`, `token=`,
`password=`) and returns the redacted payload.

It **does not mutate** the caller's dict — the dispatch payload is
preserved and `HookResult.output` carries the scrubbed copy.

```python
from atomr_agents.agent_host import redact_secrets
impl = redact_secrets(text_field="text")
# impl is an async (payload, ctx) → dict
```

## Built-in: `record_to_jsonl`

Appends `{event, payload, ts_ms}` to a JSONL file. Used to populate
`<root>/events.jsonl` so `atomr-host events tail` (M9) has something
to follow.

```python
from atomr_agents.agent_host import record_to_jsonl
impl = record_to_jsonl(target_path)
```

## Parallelism + budgets

`dispatch()` runs all matched hooks concurrently via
`asyncio.gather`. The per-hook timeout comes from
`hook.budget.get("ms", default_timeout_ms)`. A hook that times out or
raises is captured into a `HookResult` without taking down the rest of
the fan-out.

The token budget field is parsed but not yet enforced — once the
real-LLM path lands (M9) the dispatcher will subtract from the
agent's token budget per fired hook.

## Composing with a custom resolver

If you ship your own built-in hook (say `notify_slack`), pass a
resolver that recognizes its YAML `id`:

```python
from atomr_agents.agent_host import HookRegistry, default_hook_resolver

builtin = default_hook_resolver(jsonl_path=cfg.paths.events_jsonl)

def my_resolver(hook):
    if hook.call.get("id") == "notify_slack":
        return notify_slack_impl(hook.call.get("webhook"))
    return builtin(hook)

registry = HookRegistry()
registry.register_definitions(defn.hooks, my_resolver)
```

The resolver pattern keeps the substrate decoupled from the catalog
of hook implementations.
