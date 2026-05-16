# Multi-channel gateway (M7)

The host can host multiple `ChannelHarness` instances at once and
route inbound messages to the right agent. M7 ships the parser for
`<root>/AGENTS.md` and a `Gateway` that orchestrates multiple
`ChatSession`s sharing one host config.

## AGENTS.md format

```markdown
# AGENTS.md

## Defaults

- Any unmatched message: default

## Channel pins

- discord:ops-room → ops-bot
- slack:eng-incidents → incident-bot

## Peer pins

- discord:ops-room @alerts → incident-bot
- slack:eng-incidents @nightly-job → reports-bot
```

Parser tolerates both `→` and `->` arrows, mixed `:` separators,
blank lines, and prose between sections. Unknown headings reset the
parser state so unrelated content can't leak into routing rules.

## Build a router

```python
from atomr_agents.agent_host import (
    HostConfig, build_router, load_agents_md, Gateway,
)

cfg = HostConfig.load_default()
rules = load_agents_md(cfg.paths)
router = build_router(cfg, agents_md=rules)
print(router.default_agent, router.channel_pins, router.peer_pins)
```

Precedence (most-specific wins):

1. Peer pin `(channel_id, peer)`
2. Channel pin `channel_id`
3. AGENTS.md `default_agent`
4. `config.yaml` `default_agent`

## CLI

```bash
atomr-host routes
# default_agent: default
# channel pins:
#   discord:ops-room → ops-bot
# peer pins:
#   discord:ops-room @alerts → incident-bot
```

`atomr-host routes --format json` for tooling.

## Gateway lifecycle

```python
import asyncio
from atomr_agents.agent_host import HostConfig, Gateway

cfg = HostConfig.load_default()
gw = Gateway(cfg)

async def go():
    # Same agent answers two channels with separate threads.
    print(await gw.send("cli:local", "alice", "hi"))
    print(await gw.send("discord:ops-room", "alice", "what's up"))
    print(gw.open_session_ids())
    await gw.close()

asyncio.run(go())
```

- `Gateway` caches `LoadedAgent` instances by agent id so each agent's
  persona / rules / memory are loaded once and shared across sessions.
- `ChatSession`s are cached by `(channel_id, peer)` — same key
  returns the same session; different keys open a new one even for
  the same agent.
- `close()` shuts every session down in parallel via `asyncio.gather`.

## Known issue — Python 3.14 wheel

`ChannelHarness()` construction panics on the currently-built 3.14
wheel. `Gateway`-touching tests are gated by a probe that catches
`BaseException` and skip cleanly under 3.14. Pure routing-rule
parsing tests run unchanged.
