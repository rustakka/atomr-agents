# SkillCurator + CurationStrategy + events tail (M9)

M9 ships three load-bearing pieces:

1. **`EventLog`** — a JSONL log at `<root>/events.jsonl` that
   `atomr-host events tail` follows.
2. **`CurationStrategy`** — a Protocol with two built-in impls:
   `AutoPromoteCurationStrategy` (Hermes-style, **default**) and
   `HumanApprovalCurationStrategy` (Claude-Code-style, opt-in).
3. **`SkillCurator`** — observes events, drafts skill proposals,
   dispatches each to the configured strategy.

## EventLog

```python
from atomr_agents.agent_host import EventLog, HostConfig

cfg = HostConfig.load_default()
log = EventLog(cfg.paths.events_jsonl)

log.emit("tool_call_ended", agent_id="default", tool="shell.exec", ok=True)
for rec in log.read_all():
    print(rec.kind, rec.ts_ms, rec.payload)
```

`tail()` is async; consume with `async for`:

```python
import asyncio
async def follow():
    async for rec in log.tail(follow=True, poll_seconds=0.5):
        print(rec.kind, rec.payload)

# In a real host process, this runs alongside the scheduler / channels.
asyncio.run(follow())
```

CLI:

```bash
atomr-host events tail              # follows by default
atomr-host events tail --no-follow  # one-shot dump
atomr-host events emit cron_fired --agent-id default --payload '{"entry":"daily-brief"}'
```

`events emit` is a test/smoke convenience — production-side events
land through the runtime hooks dispatcher (M5) and the scheduler
(M6).

## Skill curation lifecycle

The curator drafts a `SkillProposal` when it detects a useful pattern.
What "useful" means is delegated to a **drafter** function the host
plugs in — the curator stays agnostic about heuristics so projects
can drop in an LLM-judge drafter without touching the substrate.

```python
from atomr_agents.agent_host import (
    SkillCurator, SkillProposal, EventRecord,
    AutoPromoteCurationStrategy, HumanApprovalCurationStrategy,
)

def my_drafter(batch: list[EventRecord]) -> list[SkillProposal]:
    # decide which events deserve a new/updated skill
    return [
        SkillProposal(
            agent_id="default",
            skill_id="auto_redact",
            name="Auto redact",
            body="When the user pastes a credential, refuse and explain.",
            keywords=["api_key", "password"],
            priority=8,
            rationale="seen 3 paste-then-correct cycles in the last hour",
        )
    ]

curator = SkillCurator(cfg, drafter=my_drafter)            # auto-promote default
# or opt into human approval:
curator = SkillCurator(cfg, drafter=my_drafter, strategy=HumanApprovalCurationStrategy())

outcomes = await curator.observe(events_batch)
for o in outcomes:
    print(o.accepted, o.target_path, o.reason)
```

## AutoPromoteCurationStrategy (default)

On accept:

1. Snapshots any existing `agents/<id>/skills/<skill-id>/SKILL.md`
   into `.history/<ts>.md` (enforcing `history_limit`).
2. Writes the proposal's Markdown to the live path.
3. Emits a `skill_promoted` event into the log.

Optional rubric gate:

```python
strategy = AutoPromoteCurationStrategy(min_success_rate=0.8, history_limit=10)
# ctx.metadata["success_rate"] < 0.8 → CurationOutcome(accepted=False, reason="below rubric threshold")
```

The rubric is enforced only when an *existing* skill is being
replaced — a brand-new skill always lands so the cold-start case
doesn't block on a non-existent score.

## HumanApprovalCurationStrategy (opt-in)

On accept:

1. Writes Markdown to `agents/<id>/skills/.proposed/<skill-id>/SKILL.md`.
2. Emits a `skill_proposed` event.
3. Returns `CurationOutcome(accepted=False, reason="awaiting human approval")`.

Promotion / rejection are explicit:

```bash
atomr-host skill review default                     # list pending proposals
atomr-host skill review default auto_redact --approve   # promote
atomr-host skill review default auto_redact --reject    # discard
```

Programmatic equivalents are `promote_proposal(cfg, agent_id, skill_id)`
and `reject_proposal(...)` — both snapshot the prior live version
first, so a promotion is itself reversible.

## History / revert

`AutoPromoteCurationStrategy` populates `.history/` automatically.
Manual replacements (via `promote_proposal` or `revert_skill`) also
snapshot first.

```bash
atomr-host skill history default auto_redact
# /.../skills/auto_redact/.history/1715900000000.md
# /.../skills/auto_redact/.history/1715900100000.md

atomr-host skill revert default auto_redact         # restore most-recent snapshot
atomr-host skill revert default auto_redact \
    --snapshot /.../auto_redact/.history/1715900000000.md
```

## Strategy pattern

Both built-ins satisfy the `CurationStrategy` Protocol:

```python
class CurationStrategy(Protocol):
    async def handle(self, proposal: SkillProposal, ctx: CurationCtx) -> CurationOutcome: ...
```

A third-party impl (e.g. `LlmJudgeCurationStrategy`) drops in without
changing the curator surface — pass it as `strategy=…` to
`SkillCurator`.

## Default choice

Default is **`AutoPromoteCurationStrategy()`** (Hermes-style). It
represents the bet that a host running long enough should evolve its
own skills without a human in the loop. Authors who want the
Claude-Code-style review-everything posture flip to
`HumanApprovalCurationStrategy()` in one line of `agent.yaml`:

```yaml
curation:
  strategy: human_approval
```

(Loading the YAML knob into the curator constructor is wired by
`HostRuntime` in the cross-cutting milestone; for now you swap
strategies explicitly when constructing `SkillCurator`.)
