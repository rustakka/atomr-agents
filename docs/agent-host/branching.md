# Branching / checkpoints (M10)

The host can fork an agent's working state into named branches and
snapshot it as it evolves. Branches live under
`agents/<id>/state/checkpoints/<branch-id>/` and a current pointer
lives in `state/checkpoints/CURRENT`.

## Concepts

* A **branch** is a directory under `state/checkpoints/`. Default
  branch id is `main`.
* A **checkpoint** is a JSON file at `<branch>/<ts_ms>.json` with the
  fields:
  ```json
  {
    "branch_id": "main",
    "agent_id": "default",
    "ts_ms": 1715900000000,
    "working_memory": {...},
    "thread_head": {...} | null,
    "parent_branch": null | "main"
  }
  ```
* `working_memory` is whatever dict the caller hands the host — the
  host just persists it.

## CLI

```bash
atomr-host branch ls default
# * main         latest_ts_ms=1715900000000

atomr-host branch new default experiment --from main
# forked experiment from main → .../checkpoints/experiment/1715900100000.json

atomr-host branch switch default experiment
# current branch is now `experiment` (ts=1715900100000)

atomr-host branch diff default main experiment
# {"added_keys": [...], "removed_keys": [...], ...}

atomr-host branch rm default experiment
# (refuses `main` without --force)
```

## Programmatic API

```python
from atomr_agents.agent_host import (
    HostConfig, write_checkpoint, latest_checkpoint, fork_branch, diff_branches,
)

cfg = HostConfig.load_default()
paths = cfg.paths.agent("default")

write_checkpoint(paths, "main", working_memory={"counter": 1, "topic": "intro"})
write_checkpoint(paths, "main", working_memory={"counter": 2, "topic": "intro"})

fork_branch(paths, source_branch="main", new_branch="bench")
write_checkpoint(paths, "bench", working_memory={"counter": 2, "topic": "benchmark"})

print(diff_branches(paths, "main", "bench"))
# {"added_keys": [], "removed_keys": [], "changed_keys": [{"key": "topic", "a": "intro", "b": "benchmark"}], ...}
```

## Use cases

- **Hypothesis testing**: fork a branch, mutate working memory, see
  how the agent behaves; revert by switching back. Pairs naturally
  with M9's skill curation — a curator can fork before promoting a
  proposal and roll back if downstream evals regress.
- **Experiment isolation**: long-running A/B comparisons keep
  per-branch working memory without polluting `main`.
- **Audit / replay**: every checkpoint is on disk; reading a branch's
  history shows how working memory evolved over time.

## Out of scope (for now)

- Merge / rebase between branches. Diff is one-shot; the host doesn't
  resolve conflicts.
- Sub-branches (branches-of-branches). `parent_branch` is recorded
  but there's no cascading delete or hierarchy view.
- Compaction of identical adjacent checkpoints. `prune_branch` keeps
  the N most recent — that's enough for the M10 use cases.
