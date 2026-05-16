"""SkillCurator + CurationStrategy (M9).

The curator observes :class:`~atomr_agents.agent_host.events.EventRecord`
batches, drafts :class:`SkillProposal` instances via a pluggable drafter,
and hands each proposal to a :class:`CurationStrategy`.

Two built-in strategies ship:

- :class:`AutoPromoteCurationStrategy` — default; Hermes-style. Writes
  the new SKILL.md directly to the agent's live ``skills/<id>/`` dir.
  Snapshots any prior version into ``.history/<ts>.md`` so a revert is
  always a one-step operation. Optional rubric gate
  (``min_success_rate``) blocks promotion when the current skill is
  performing well enough that the proposed replacement should clear a
  threshold first.
- :class:`HumanApprovalCurationStrategy` — opt-in. Writes proposed
  SKILL.md to ``skills/.proposed/<id>/SKILL.md``. Promotion is a
  separate :func:`promote_proposal` call (or ``atomr-host skill review``
  via the CLI).

Helpers :func:`promote_proposal` / :func:`reject_proposal` /
:func:`list_proposals` / :func:`list_history` / :func:`revert_skill`
round out the curator-substrate API.
"""

from __future__ import annotations

import asyncio
import inspect
import json
import shutil
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Awaitable, Callable, Protocol, runtime_checkable

from .config import HostConfig
from .errors import AgentHostError, AgentSpecError
from .events import EventLog, EventRecord
from .layout import AgentPaths

try:
    import yaml  # type: ignore[import-untyped]

    _yaml_available = True
except ImportError:  # pragma: no cover
    yaml = None  # type: ignore[assignment]
    _yaml_available = False


__all__ = [
    "SkillProposal",
    "CurationOutcome",
    "CurationCtx",
    "CurationStrategy",
    "AutoPromoteCurationStrategy",
    "HumanApprovalCurationStrategy",
    "SkillCurator",
    "ProposalDrafter",
    "promote_proposal",
    "reject_proposal",
    "list_proposals",
    "list_history",
    "revert_skill",
]


# ---------- dataclasses ---------------------------------------------------


@dataclass(frozen=True)
class SkillProposal:
    """A skill the curator wants to add or replace.

    Maps 1:1 onto a SKILL.md file: ``to_markdown`` renders the file the
    strategy ends up writing.
    """

    agent_id: str
    skill_id: str
    name: str
    body: str
    """SKILL.md body — the ``instruction_fragment`` once loaded."""
    keywords: list[str] = field(default_factory=list)
    priority: int = 5
    tool_overlay: list[str] = field(default_factory=list)
    rationale: str = ""
    """Why the curator believes this skill helps. Surfaced in events."""

    def to_markdown(self) -> str:
        """Render as a SKILL.md file with YAML frontmatter + body.

        Frontmatter mirrors what :func:`scaffold_skill` writes so that
        the loader's parser sees an identically-shaped document either
        way.
        """
        if not _yaml_available:
            raise AgentHostError(
                "PyYAML is required to render SkillProposal markdown — "
                "install atomr-agents[host]"
            )
        frontmatter: dict[str, Any] = {
            "name": self.name,
            "priority": int(self.priority),
            "keywords": list(self.keywords),
            "tool_overlay": list(self.tool_overlay),
            "memory_namespace": [],
        }
        yaml_block = yaml.safe_dump(frontmatter, sort_keys=False).rstrip("\n")
        body = self.body.strip()
        return f"---\n{yaml_block}\n---\n\n{body}\n" if body else f"---\n{yaml_block}\n---\n"


@dataclass(frozen=True)
class CurationOutcome:
    """Result of a strategy handling a proposal."""

    accepted: bool
    """True iff the skill is now live on disk."""
    target_path: Path | None
    """Final path of the SKILL.md (either live or ``.proposed/``)."""
    reason: str = ""
    """Human-readable explanation. Always populated on rejection."""
    history_path: Path | None = None
    """Snapshot of the prior live version, when one existed."""


@dataclass
class CurationCtx:
    """Per-call context handed to :meth:`CurationStrategy.handle`."""

    config: HostConfig
    events: EventLog | None = None
    metadata: dict = field(default_factory=dict)
    """Strategy-specific knobs, e.g. ``{"success_rate": 0.72}``."""


@runtime_checkable
class CurationStrategy(Protocol):
    """Where a proposal lands. Pluggable per-agent via ``agent.yaml``."""

    async def handle(
        self, proposal: SkillProposal, ctx: CurationCtx
    ) -> CurationOutcome: ...


# ---------- internal path helpers -----------------------------------------


def _agent_paths(config: HostConfig, agent_id: str) -> AgentPaths:
    return config.paths.agent(agent_id)


def _require_agent_dir(paths: AgentPaths) -> None:
    if not paths.dir.is_dir():
        raise AgentSpecError(
            f"agent `{paths.agent_id}` has no directory under {paths.dir.parent}"
        )


def _live_skill_dir(paths: AgentPaths, skill_id: str) -> Path:
    return paths.skills_dir / skill_id


def _live_skill_file(paths: AgentPaths, skill_id: str) -> Path:
    return _live_skill_dir(paths, skill_id) / "SKILL.md"


def _proposed_skill_dir(paths: AgentPaths, skill_id: str) -> Path:
    return paths.skills_dir / ".proposed" / skill_id


def _proposed_skill_file(paths: AgentPaths, skill_id: str) -> Path:
    return _proposed_skill_dir(paths, skill_id) / "SKILL.md"


def _history_dir(paths: AgentPaths, skill_id: str) -> Path:
    return _live_skill_dir(paths, skill_id) / ".history"


def _snapshot_live(
    paths: AgentPaths,
    skill_id: str,
    *,
    history_limit: int | None = None,
) -> Path | None:
    """Copy the current live SKILL.md (if any) into ``.history/<ts>.md``.

    Returns the snapshot path, or ``None`` when there's nothing to snapshot.
    Prunes oldest snapshots when ``history_limit`` is set.
    """
    live = _live_skill_file(paths, skill_id)
    if not live.is_file():
        return None
    hist_dir = _history_dir(paths, skill_id)
    hist_dir.mkdir(parents=True, exist_ok=True)
    ts = int(time.time() * 1000)
    # Avoid collisions when multiple snapshots fire in the same millisecond.
    target = hist_dir / f"{ts}.md"
    if target.exists():
        suffix = 1
        while True:
            candidate = hist_dir / f"{ts}-{suffix}.md"
            if not candidate.exists():
                target = candidate
                break
            suffix += 1
    shutil.copy2(live, target)

    if history_limit is not None and history_limit >= 0:
        snapshots = sorted(
            (p for p in hist_dir.iterdir() if p.is_file() and p.suffix == ".md"),
            key=lambda p: (p.stat().st_mtime, p.name),
        )
        while len(snapshots) > history_limit:
            victim = snapshots.pop(0)
            try:
                victim.unlink()
            except FileNotFoundError:  # pragma: no cover
                pass
    return target


def _write_skill_md(target: Path, proposal: SkillProposal) -> None:
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(proposal.to_markdown(), encoding="utf-8")


# ---------- strategies ----------------------------------------------------


class AutoPromoteCurationStrategy:
    """Default Hermes-style strategy.

    Writes the proposal directly to the agent's live skills directory.
    Prior versions are snapshotted into ``.history/`` so reverting a
    bad promotion is trivial.
    """

    def __init__(
        self,
        *,
        min_success_rate: float | None = None,
        history_limit: int = 20,
    ) -> None:
        self._min_success_rate = min_success_rate
        self._history_limit = int(history_limit)

    @property
    def min_success_rate(self) -> float | None:
        return self._min_success_rate

    @property
    def history_limit(self) -> int:
        return self._history_limit

    async def handle(
        self, proposal: SkillProposal, ctx: CurationCtx
    ) -> CurationOutcome:
        paths = _agent_paths(ctx.config, proposal.agent_id)
        _require_agent_dir(paths)

        target = _live_skill_file(paths, proposal.skill_id)
        target_exists = target.is_file()

        # Rubric gate. Only enforced when there's an existing skill to
        # protect — a brand-new skill has nothing to beat.
        if (
            target_exists
            and self._min_success_rate is not None
            and float(ctx.metadata.get("success_rate", 1.0)) < float(self._min_success_rate)
        ):
            return CurationOutcome(
                accepted=False,
                target_path=target,
                reason="below rubric threshold",
            )

        snapshot = (
            _snapshot_live(paths, proposal.skill_id, history_limit=self._history_limit)
            if target_exists
            else None
        )
        _write_skill_md(target, proposal)

        if ctx.events is not None:
            ctx.events.emit(
                "skill_promoted",
                agent_id=proposal.agent_id,
                skill_id=proposal.skill_id,
                name=proposal.name,
                rationale=proposal.rationale,
                strategy="auto_promote",
                history_path=str(snapshot) if snapshot else None,
            )

        return CurationOutcome(
            accepted=True,
            target_path=target,
            history_path=snapshot,
            reason="auto-promoted",
        )


class HumanApprovalCurationStrategy:
    """Stage proposals under ``skills/.proposed/<id>/`` for review.

    Promotion is a separate step (:func:`promote_proposal`), so the
    outcome here is always ``accepted=False``.
    """

    async def handle(
        self, proposal: SkillProposal, ctx: CurationCtx
    ) -> CurationOutcome:
        paths = _agent_paths(ctx.config, proposal.agent_id)
        _require_agent_dir(paths)
        target = _proposed_skill_file(paths, proposal.skill_id)
        _write_skill_md(target, proposal)

        if ctx.events is not None:
            ctx.events.emit(
                "skill_proposed",
                agent_id=proposal.agent_id,
                skill_id=proposal.skill_id,
                name=proposal.name,
                rationale=proposal.rationale,
                strategy="human_approval",
                target_path=str(target),
            )

        return CurationOutcome(
            accepted=False,
            target_path=target,
            reason="awaiting human approval",
        )


# ---------- proposal lifecycle helpers ------------------------------------


def promote_proposal(
    config: HostConfig, agent_id: str, skill_id: str
) -> CurationOutcome:
    """Move ``.proposed/<id>/SKILL.md`` to ``skills/<id>/SKILL.md``.

    Snapshots the prior live version (if any) into ``.history/``. Raises
    :class:`AgentSpecError` when the proposal is missing.
    """
    paths = _agent_paths(config, agent_id)
    _require_agent_dir(paths)
    proposed = _proposed_skill_file(paths, skill_id)
    if not proposed.is_file():
        raise AgentSpecError(
            f"no proposal at {proposed} — nothing to promote"
        )
    snapshot = _snapshot_live(paths, skill_id)
    target = _live_skill_file(paths, skill_id)
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(proposed, target)
    # Remove the proposed file + dir if empty.
    try:
        proposed.unlink()
    except FileNotFoundError:  # pragma: no cover
        pass
    parent = proposed.parent
    try:
        parent.rmdir()
    except OSError:
        pass
    return CurationOutcome(
        accepted=True,
        target_path=target,
        history_path=snapshot,
        reason="promoted",
    )


def reject_proposal(
    config: HostConfig, agent_id: str, skill_id: str
) -> CurationOutcome:
    """Delete ``.proposed/<id>/SKILL.md``."""
    paths = _agent_paths(config, agent_id)
    _require_agent_dir(paths)
    proposed = _proposed_skill_file(paths, skill_id)
    if not proposed.is_file():
        raise AgentSpecError(
            f"no proposal at {proposed} — nothing to reject"
        )
    proposed.unlink()
    parent = proposed.parent
    try:
        parent.rmdir()
    except OSError:
        pass
    return CurationOutcome(
        accepted=False,
        target_path=None,
        reason="rejected",
    )


def list_proposals(config: HostConfig, agent_id: str) -> list[Path]:
    """Sorted paths of all SKILL.md files currently under ``.proposed/``."""
    paths = _agent_paths(config, agent_id)
    proposed_root = paths.skills_dir / ".proposed"
    if not proposed_root.is_dir():
        return []
    out: list[Path] = []
    for child in sorted(proposed_root.iterdir()):
        if not child.is_dir():
            continue
        candidate = child / "SKILL.md"
        if candidate.is_file():
            out.append(candidate)
    return out


def list_history(
    config: HostConfig, agent_id: str, skill_id: str
) -> list[Path]:
    """Sorted oldest→newest history snapshots for ``<skill_id>``."""
    paths = _agent_paths(config, agent_id)
    hist_dir = _history_dir(paths, skill_id)
    if not hist_dir.is_dir():
        return []
    snapshots = [
        p for p in hist_dir.iterdir() if p.is_file() and p.suffix == ".md"
    ]
    snapshots.sort(key=lambda p: (p.stat().st_mtime, p.name))
    return snapshots


def revert_skill(
    config: HostConfig,
    agent_id: str,
    skill_id: str,
    *,
    target: Path | None = None,
) -> CurationOutcome:
    """Restore the most-recent (or named) history snapshot.

    Snapshots the current live SKILL.md into ``.history/`` before
    overwriting so the revert itself is reversible.
    """
    paths = _agent_paths(config, agent_id)
    _require_agent_dir(paths)
    history = list_history(config, agent_id, skill_id)
    if not history:
        raise AgentSpecError(
            f"no history snapshots for skill `{skill_id}` under agent `{agent_id}`"
        )
    if target is None:
        snapshot_src = history[-1]
    else:
        target_path = Path(target)
        if not target_path.is_file() or target_path not in history:
            raise AgentSpecError(
                f"history snapshot {target_path} not found for skill `{skill_id}`"
            )
        snapshot_src = target_path

    # Snapshot the current live version before overwriting.
    snapshot = _snapshot_live(paths, skill_id)

    live = _live_skill_file(paths, skill_id)
    live.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(snapshot_src, live)
    return CurationOutcome(
        accepted=True,
        target_path=live,
        history_path=snapshot,
        reason=f"reverted to {snapshot_src.name}",
    )


# ---------- curator -------------------------------------------------------


ProposalDrafter = Callable[
    [list[EventRecord]],
    "list[SkillProposal] | Awaitable[list[SkillProposal]]",
]


def _empty_drafter(_batch: list[EventRecord]) -> list[SkillProposal]:
    return []


class SkillCurator:
    """Observes events → drafts proposals → dispatches via a strategy.

    The drafter is intentionally simple-by-default: in tests and
    production, callers plug in their own (LLM-backed or heuristic)
    drafter. The default returns ``[]`` so the curator is safe to wire
    into a long-running loop without spamming the strategy.
    """

    def __init__(
        self,
        config: HostConfig,
        *,
        strategy: CurationStrategy | None = None,
        drafter: ProposalDrafter | None = None,
        events: EventLog | None = None,
    ) -> None:
        self._config = config
        self._strategy: CurationStrategy = strategy or AutoPromoteCurationStrategy()
        self._drafter: ProposalDrafter = drafter or _empty_drafter
        self._events = events

    @property
    def config(self) -> HostConfig:
        return self._config

    @property
    def strategy(self) -> CurationStrategy:
        return self._strategy

    @strategy.setter
    def strategy(self, strategy: CurationStrategy) -> None:
        self._strategy = strategy

    @property
    def drafter(self) -> ProposalDrafter:
        return self._drafter

    @drafter.setter
    def drafter(self, drafter: ProposalDrafter) -> None:
        self._drafter = drafter or _empty_drafter

    @property
    def events(self) -> EventLog | None:
        return self._events

    async def observe(
        self, batch: list[EventRecord]
    ) -> list[CurationOutcome]:
        """Run drafter over ``batch`` → dispatch via strategy → collect outcomes.

        Per-proposal exceptions are captured into
        ``CurationOutcome(accepted=False, reason=repr(exc))`` so a single
        bad proposal can't poison the rest of the batch.
        """
        drafted = self._drafter(list(batch))
        if inspect.isawaitable(drafted):
            drafted = await drafted
        proposals: list[SkillProposal] = list(drafted or [])

        ctx = CurationCtx(config=self._config, events=self._events)
        outcomes: list[CurationOutcome] = []
        for proposal in proposals:
            try:
                outcome = await self._strategy.handle(proposal, ctx)
            except Exception as exc:  # noqa: BLE001 - capture everything
                outcomes.append(
                    CurationOutcome(
                        accepted=False,
                        target_path=None,
                        reason=repr(exc),
                    )
                )
                continue
            outcomes.append(outcome)
        return outcomes
