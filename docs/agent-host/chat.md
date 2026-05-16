# `atomr-host chat` — local CLI channel (M2)

After `atomr-host init` you can chat with an agent locally:

```bash
atomr-host chat default
# atomr-host chat — agent: default (gpt-4o)
# Type your message and press enter. /quit to exit.
# > hello
# [default] A helpful general-purpose assistant.
# user: hello
# rules: 3 | memory facts: 1 | skills: 0
# > /quit
```

## What's happening underneath

M2 wires three existing pieces together:

1. **`ChannelHarness`** (`crates/channel-harness`) — the existing
   atomr-agents transport. The host instantiates one per chat session
   and attaches an in-memory provider as the stdio carrier.
2. **A sync `Callable`** built by
   :func:`atomr_agents.agent_host.chat.build_chat_callable`. The
   callable closes over the :class:`LoadedAgent` (persona / rules /
   memory / skills) and renders a deterministic preview response.
3. **A thread** opened on the channel. The harness routes inbound
   messages to the bound callable and emits `turn_started` →
   `turn_completed` → `message_sent` events.

The plumbing matches `python/atomr_agents/tests/test_channel.py` —
the host adds the per-agent context wiring on top.

## Why a preview responder?

M2 ships a deterministic responder so that the channel/thread/router
plumbing can be tested end-to-end without an API key. The responder
renders:

```
[<agent-id>] <persona identity>
user: <message>
rules: N | memory facts: M | skills: K
```

M9 swaps the responder for a real `InferenceClient` call. Because
`build_chat_callable` accepts a custom `responder` parameter, you can
already point it at a real LLM today:

```python
from atomr_agents.agent_host import build_chat_callable, AgentLoader, HostConfig

def my_llm(loaded, user_msg):
    # call your favorite SDK here, returning the reply text
    return openai_call(loaded, user_msg)

loaded = AgentLoader(HostConfig.load_default()).load("default")
cb = build_chat_callable(loaded, responder=my_llm)
```

## AgentRouter

`AgentRouter` maps `(channel_id, peer) → agent_id`. M2 ships three
modes:

- `default_agent` fallback (taken from `config.yaml`).
- `pin_channel(channel_id, agent_id)` — channel-wide pin.
- `pin_peer(channel_id, peer, agent_id)` — most-specific wins.

```python
from atomr_agents.agent_host import AgentRouter

r = AgentRouter(default_agent="default")
r.pin_channel("discord:fleet-ops", "ops-bot")
r.pin_peer("discord:fleet-ops", "@alerts", "incident-bot")
```

M7 replaces this with an AGENTS.md-driven implementation; the API
above is the stable surface — only the routing data source changes.

## Thread persistence

Each turn appends a record to
`agents/<id>/state/threads/<channel>/<thread>.jsonl`:

```jsonl
{"kind":"thread_opened","agent_id":"default","channel_id":"cli:local","peer":"user","thread_id":"cli:local#user"}
{"kind":"user_message","msg_id":"cli-1","text":"hello"}
{"kind":"agent_reply","msg_id":"cli-1","text":"[default] ...\nuser: hello\nrules: 3 | memory facts: 1 | skills: 0"}
{"kind":"thread_closed"}
```

Pass `--no-persist` (or `persist=False` programmatically) to skip the
disk log — useful in ephemeral CI runs.

The on-disk JSONL is the host's *audit* log. The harness still owns
the *live* thread state in memory via its `ChannelStore`. M10 unifies
these into a single checkpointable representation.

## Known issue — Python 3.14 wheel

The currently-built 3.14 `_native` wheel panics on `ChannelHarness()`
construction (`there is no reactor running, must be called from the
context of a Tokio 1.x runtime`). Chat tests skip gracefully under
3.14 and run under 3.12. Fix tracked alongside the maturin matrix
update in the cross-cutting milestone.
