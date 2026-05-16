# Registry pull (M11)

The atomr-agents `Registry` (`crates/registry`) is an in-memory index
of published artifacts (`tool_set`, `skill`, `persona`, `agent`,
`workflow`, `harness`, `channel`). M11 adds a thin disk cache so the
host can resolve those artifacts without re-fetching.

## Disk layout

```
<root>/registry/
  skill/
    summarize/
      0.1.0.json
      0.2.0.json
  persona/
    pragmatic-engineer/
      1.0.0.json
```

Each JSON file holds:

```json
{
  "kind": "skill",
  "id": "summarize",
  "version": "0.1.0",
  "payload": {...},
  "cached_at_ms": 1715900000000
}
```

## CLI

```bash
atomr-host registry ls
# skill:summarize@0.1.0     (.../registry/skill/summarize/0.1.0.json)
# skill:summarize@0.2.0     (.../registry/skill/summarize/0.2.0.json)
# persona:pragmatic-engineer@1.0.0  (...)

atomr-host registry ls --kind skill

atomr-host registry resolve skill:summarize@0.2.0
# {"slug":"skill:summarize@0.2.0","path":"...","payload":{...}}
```

## Programmatic API

```python
from atomr_agents.agent_host import (
    HostConfig, cache_artifact, pull_artifact, resolve_artifact,
    list_artifacts, verify_cache, parse_slug,
)

cfg = HostConfig.load_default()

# Cache a payload directly.
cache_artifact(cfg.paths,
    kind="skill", id="summarize", version="0.1.0",
    payload={"name": "Summarize", "priority": 7, ...})

# Pull from a duck-typed `registry` (anything with `.get(kind,id,version)`
# and `.latest(kind,id)`). The native Registry facade satisfies the shape.
from atomr_agents import _native
art = pull_artifact(cfg.paths, _native.registry.Registry(),
                    kind="skill", id="summarize")   # version=None → latest

# Resolve without re-fetching.
art = resolve_artifact(cfg.paths, kind="skill", id="summarize", version="0.2.0")
print(art.slug, art.payload)
```

## Verify

`verify_cache` cross-checks the on-disk cache against a fresh registry
lookup — useful for CI. Returns `[]` when consistent, else a list of
`(artifact, "missing" | "mismatch")` tuples.

```python
from atomr_agents.agent_host import verify_cache
diffs = verify_cache(cfg.paths, my_registry)
if diffs:
    for art, reason in diffs:
        print(art.slug, reason)
    raise SystemExit(1)
```

## Out of scope

- Over-the-network pull. M11 takes any duck-typed `Registry` object;
  the source is the caller's responsibility. A future revision could
  add an HTTP-backed `RemoteRegistry` adapter.
- Semver-aware resolution. `resolve_artifact(version=None)` picks the
  lexicographically newest cached version. For 1-digit-segment versions
  that matches semver; for irregular tag schemes (e.g. `2025.05.15`)
  the rules still hold. Calls that need strict semver should pass an
  explicit `version`.
- Cache eviction. M11 caches forever; bound the directory size with
  `delete_artifact` or just `rm` what you don't want.
