# Scheduler + crons (M6)

The host owns a tiny tokio-friendly scheduler that fires registered
callables on a per-entry schedule. Entries live on disk at
`<root>/crons/<id>.yaml`; the runtime parses them into `CronEntry`
dataclasses and registers them with a `Scheduler`.

## Cron file format

```yaml
# <root>/crons/daily-brief.yaml
expression: every:24h
call:
  kind: builtin
  id: noop                  # or {kind: skill, id: <skill-id>} once skills are wired
input:
  topic: morning standup
enabled: true
```

## Supported expressions (M6)

| Expression  | Meaning             |
|-------------|---------------------|
| `every:30s` | every 30 seconds    |
| `every:5m`  | every 5 minutes     |
| `every:1h`  | every hour          |
| `every:1d`  | every 24 hours      |

Full 5-field cron syntax (`*/5 * * * *`) is a planned follow-up. The
M6 surface intentionally keeps the parser tiny so the dependency set
stays at PyYAML only.

## CLI

```bash
atomr-host cron add daily-brief \
    --when 'every:1h' \
    --call '{"kind":"builtin","id":"noop"}' \
    --input '{"topic":"standup"}'
# wrote /.../crons/daily-brief.yaml

atomr-host cron ls
# daily-brief  when=every:1h  call=noop  enabled=True

atomr-host cron rm daily-brief --force
```

## Programmatic API

```python
import asyncio, time
from atomr_agents.agent_host import HostConfig, Scheduler, load_crons, default_cron_resolver

cfg = HostConfig.load_default()
entries = load_crons(cfg)

scheduler = Scheduler()
resolver = default_cron_resolver()
for e in entries:
    impl = resolver(e)
    if impl is not None:
        scheduler.register(e, impl)

# Drive the loop for 5 seconds — for production this becomes a long-lived
# task in HostRuntime.run().
results = asyncio.run(scheduler.tick_until(until_ts=time.monotonic() + 5))
for r in results:
    print(r.entry_id, r.ok, r.duration_ms)
```

`Scheduler` accepts a `clock` argument so tests can drive time forward
manually:

```python
clock = lambda: tick.current
scheduler = Scheduler(clock=clock)
scheduler.register(entry, impl)
tick.current = 65    # advance past the 60s interval
asyncio.run(scheduler.fire_due())  # fires now
```

## Semantics

- Each entry's interval is parsed once at `register()` and cached.
- `fire_due()` runs **all** due entries concurrently via
  `asyncio.gather`.
- Failures (timeouts, exceptions) are captured into
  `CronFireResult(ok=False, error=...)`; one failing entry doesn't
  prevent others from firing.
- Disabled entries are skipped without error.
- After a fire (success or failure), the next-fire timestamp is
  rescheduled — failures don't park the entry.

## How M6 fits the bigger picture

Today `Scheduler` is driven from a one-shot `tick_until` in tests and
CLI. The cross-cutting milestone wires it into a long-lived host
runtime so the loop runs as long as `atomr-host run` is alive,
emitting events into the JSONL log (M9) on every fire.
