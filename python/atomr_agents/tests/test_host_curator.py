"""Tests for the M9 EventLog + SkillCurator + CurationStrategy layer.

These tests are pure-Python; they do not depend on ``atomr_agents._native``
and drive async code with ``asyncio.run`` so we don't need
``pytest-asyncio``.
"""

from __future__ import annotations

import asyncio
import json
import time
from pathlib import Path

import pytest

pytest.importorskip("yaml")

from atomr_agents.agent_host.config import HostConfig
from atomr_agents.agent_host.curator import (
    AutoPromoteCurationStrategy,
    CurationCtx,
    CurationOutcome,
    HumanApprovalCurationStrategy,
    SkillCurator,
    SkillProposal,
    list_history,
    list_proposals,
    promote_proposal,
    reject_proposal,
    revert_skill,
)
from atomr_agents.agent_host.errors import AgentSpecError
from atomr_agents.agent_host.events import EventLog, EventRecord
from atomr_agents.agent_host.layout import HostPaths


# ---------- helpers --------------------------------------------------------


def _host_config(tmp_path: Path) -> HostConfig:
    paths = HostPaths(root=tmp_path)
    paths.ensure()
    return HostConfig(paths=paths)


def _make_agent(tmp_path: Path, agent_id: str = "alpha") -> HostConfig:
    cfg = _host_config(tmp_path)
    cfg.paths.agent(agent_id).ensure()
    return cfg


def _proposal(
    *,
    agent_id: str = "alpha",
    skill_id: str = "summarize",
    body: str = "Summarize succinctly.",
    keywords: list[str] | None = None,
    priority: int = 5,
) -> SkillProposal:
    return SkillProposal(
        agent_id=agent_id,
        skill_id=skill_id,
        name="Summarize",
        body=body,
        keywords=keywords or ["summarize", "tldr"],
        priority=priority,
        rationale="user asked for TL;DR three times this hour",
    )


# ---------- EventLog -------------------------------------------------------


def test_event_log_append_and_read_all_round_trip(tmp_path: Path) -> None:
    log = EventLog(tmp_path / "events.jsonl")
    rec1 = EventRecord(kind="tool_call_ended", ts_ms=1.0, agent_id="a", payload={"ok": True})
    rec2 = EventRecord(kind="skill_promoted", ts_ms=2.0, agent_id="a", payload={"id": "x"})
    log.append(rec1)
    log.append(rec2)

    out = log.read_all()
    assert [r.kind for r in out] == ["tool_call_ended", "skill_promoted"]
    assert out[0].payload == {"ok": True}
    assert out[1].payload == {"id": "x"}
    assert out[0].ts_ms == 1.0


def test_event_log_emit_builds_record_and_appends(tmp_path: Path) -> None:
    log = EventLog(tmp_path / "events.jsonl")
    rec = log.emit("cron_fired", agent_id="a", cron_id="hourly")
    assert rec.kind == "cron_fired"
    assert rec.agent_id == "a"
    assert rec.payload == {"cron_id": "hourly"}

    on_disk = log.read_all()
    assert len(on_disk) == 1
    assert on_disk[0].kind == "cron_fired"
    assert on_disk[0].payload == {"cron_id": "hourly"}


def test_event_log_read_all_handles_missing_file(tmp_path: Path) -> None:
    log = EventLog(tmp_path / "no-such.jsonl")
    assert log.read_all() == []


def test_event_log_tail_no_follow_yields_existing_lines(tmp_path: Path) -> None:
    log = EventLog(tmp_path / "events.jsonl")
    log.emit("a", agent_id="x")
    log.emit("b", agent_id="x")
    log.emit("c", agent_id="x")

    async def _collect() -> list[EventRecord]:
        out: list[EventRecord] = []
        async for rec in log.tail(follow=False):
            out.append(rec)
        return out

    out = asyncio.run(_collect())
    assert [r.kind for r in out] == ["a", "b", "c"]


def test_event_log_tail_follow_yields_new_lines(tmp_path: Path) -> None:
    log = EventLog(tmp_path / "events.jsonl")
    log.emit("seed", agent_id="x")

    async def _runner() -> list[str]:
        received: list[str] = []

        async def _reader() -> None:
            async for rec in log.tail(follow=True, poll_seconds=0.02):
                received.append(rec.kind)
                if len(received) >= 2:
                    return

        async def _writer() -> None:
            await asyncio.sleep(0.05)
            log.emit("fresh", agent_id="x")

        try:
            await asyncio.wait_for(
                asyncio.gather(_reader(), _writer()),
                timeout=2.0,
            )
        except asyncio.TimeoutError:  # pragma: no cover
            pytest.fail(f"tail did not pick up new line; received={received!r}")
        return received

    received = asyncio.run(_runner())
    assert received == ["seed", "fresh"]


def test_event_log_tail_cancellation_is_clean(tmp_path: Path) -> None:
    log = EventLog(tmp_path / "events.jsonl")
    log.emit("only", agent_id="x")

    async def _runner() -> int:
        received: list[EventRecord] = []

        async def _reader() -> None:
            async for rec in log.tail(follow=True, poll_seconds=0.02):
                received.append(rec)

        task = asyncio.create_task(_reader())
        await asyncio.sleep(0.1)
        task.cancel()
        try:
            await task
        except asyncio.CancelledError:
            pass
        return len(received)

    count = asyncio.run(_runner())
    assert count == 1


# ---------- SkillProposal.to_markdown -------------------------------------


def test_proposal_to_markdown_contains_frontmatter_and_body() -> None:
    p = _proposal(keywords=["summarize", "digest"], priority=7, body="Be concise.")
    md = p.to_markdown()
    # Frontmatter bracketed by ---
    assert md.startswith("---\n")
    assert "\n---\n" in md[4:]
    # Required fields
    assert "name: Summarize" in md
    assert "priority: 7" in md
    assert "- summarize" in md
    assert "- digest" in md
    # Body
    assert "Be concise." in md


# ---------- AutoPromoteCurationStrategy -----------------------------------


def test_auto_promote_first_call_writes_target_no_history(tmp_path: Path) -> None:
    cfg = _make_agent(tmp_path)
    events = EventLog(cfg.paths.events_jsonl)
    strategy = AutoPromoteCurationStrategy()

    proposal = _proposal()
    outcome = asyncio.run(
        strategy.handle(proposal, CurationCtx(config=cfg, events=events))
    )

    assert outcome.accepted is True
    assert outcome.history_path is None
    assert outcome.target_path is not None
    assert outcome.target_path.is_file()
    content = outcome.target_path.read_text(encoding="utf-8")
    assert "Summarize succinctly." in content

    # Should have emitted skill_promoted.
    on_disk = events.read_all()
    assert any(r.kind == "skill_promoted" for r in on_disk)
    promoted = next(r for r in on_disk if r.kind == "skill_promoted")
    assert promoted.agent_id == "alpha"
    assert promoted.payload.get("skill_id") == "summarize"


def test_auto_promote_second_call_creates_history_snapshot(tmp_path: Path) -> None:
    cfg = _make_agent(tmp_path)
    strategy = AutoPromoteCurationStrategy()

    asyncio.run(
        strategy.handle(
            _proposal(body="v1 body"), CurationCtx(config=cfg)
        )
    )
    # Sleep so the file mtime advances (Linux mtime can be coarse).
    time.sleep(0.01)
    outcome = asyncio.run(
        strategy.handle(
            _proposal(body="v2 body — bigger and better"),
            CurationCtx(config=cfg),
        )
    )

    assert outcome.accepted is True
    assert outcome.history_path is not None
    assert outcome.history_path.is_file()
    history_content = outcome.history_path.read_text(encoding="utf-8")
    assert "v1 body" in history_content
    # And the live file holds the new version.
    live_content = outcome.target_path.read_text(encoding="utf-8")
    assert "v2 body" in live_content


def test_auto_promote_below_rubric_rejects(tmp_path: Path) -> None:
    cfg = _make_agent(tmp_path)
    strategy = AutoPromoteCurationStrategy(min_success_rate=0.9)

    # Seed the live skill so the rubric gate has something to protect.
    asyncio.run(
        strategy.handle(_proposal(body="seed"), CurationCtx(config=cfg))
    )

    outcome = asyncio.run(
        strategy.handle(
            _proposal(body="should be rejected"),
            CurationCtx(config=cfg, metadata={"success_rate": 0.5}),
        )
    )
    assert outcome.accepted is False
    assert "rubric" in outcome.reason
    # Live SKILL.md must still be the seed body.
    assert "seed" in outcome.target_path.read_text(encoding="utf-8")


def test_auto_promote_history_limit_prunes_oldest(tmp_path: Path) -> None:
    cfg = _make_agent(tmp_path)
    strategy = AutoPromoteCurationStrategy(history_limit=2)

    for n in range(4):
        asyncio.run(
            strategy.handle(
                _proposal(body=f"version {n}"), CurationCtx(config=cfg)
            )
        )
        # Force mtimes apart so pruning ordering is deterministic.
        time.sleep(0.01)

    hist = list_history(cfg, "alpha", "summarize")
    assert len(hist) == 2
    # Newest snapshot should hold "version 2" (version 3 is live, never snapshotted).
    newest_content = hist[-1].read_text(encoding="utf-8")
    assert "version 2" in newest_content


# ---------- HumanApprovalCurationStrategy ---------------------------------


def test_human_approval_writes_to_proposed_dir(tmp_path: Path) -> None:
    cfg = _make_agent(tmp_path)
    events = EventLog(cfg.paths.events_jsonl)
    strategy = HumanApprovalCurationStrategy()

    proposal = _proposal()
    outcome = asyncio.run(
        strategy.handle(proposal, CurationCtx(config=cfg, events=events))
    )

    assert outcome.accepted is False
    assert outcome.reason == "awaiting human approval"
    assert outcome.target_path is not None
    expected = (
        cfg.paths.agent("alpha").skills_dir
        / ".proposed"
        / "summarize"
        / "SKILL.md"
    )
    assert outcome.target_path == expected
    assert expected.is_file()
    # And the live skill does NOT exist yet.
    assert not (cfg.paths.agent("alpha").skills_dir / "summarize" / "SKILL.md").is_file()

    # skill_proposed event emitted.
    on_disk = events.read_all()
    assert [r.kind for r in on_disk] == ["skill_proposed"]


# ---------- promote_proposal / reject_proposal ----------------------------


def test_promote_proposal_moves_proposed_to_live_with_history(tmp_path: Path) -> None:
    cfg = _make_agent(tmp_path)

    # Seed a live skill first.
    asyncio.run(
        AutoPromoteCurationStrategy().handle(
            _proposal(body="live v1"), CurationCtx(config=cfg)
        )
    )
    # Stage a proposed replacement via human-approval.
    asyncio.run(
        HumanApprovalCurationStrategy().handle(
            _proposal(body="proposed v2"), CurationCtx(config=cfg)
        )
    )
    time.sleep(0.01)

    outcome = promote_proposal(cfg, "alpha", "summarize")
    assert outcome.accepted is True
    assert outcome.target_path is not None
    assert "proposed v2" in outcome.target_path.read_text(encoding="utf-8")
    # Snapshot taken of the prior live version.
    assert outcome.history_path is not None
    assert "live v1" in outcome.history_path.read_text(encoding="utf-8")
    # .proposed dir cleaned up.
    proposed = (
        cfg.paths.agent("alpha").skills_dir
        / ".proposed"
        / "summarize"
        / "SKILL.md"
    )
    assert not proposed.exists()


def test_promote_proposal_with_no_proposal_raises(tmp_path: Path) -> None:
    cfg = _make_agent(tmp_path)
    with pytest.raises(AgentSpecError):
        promote_proposal(cfg, "alpha", "nope")


def test_reject_proposal_removes_proposed_file(tmp_path: Path) -> None:
    cfg = _make_agent(tmp_path)
    asyncio.run(
        HumanApprovalCurationStrategy().handle(
            _proposal(body="never live"), CurationCtx(config=cfg)
        )
    )
    proposed = (
        cfg.paths.agent("alpha").skills_dir
        / ".proposed"
        / "summarize"
        / "SKILL.md"
    )
    assert proposed.is_file()

    outcome = reject_proposal(cfg, "alpha", "summarize")
    assert outcome.accepted is False
    assert outcome.reason == "rejected"
    assert not proposed.exists()


# ---------- list_proposals / list_history ---------------------------------


def test_list_proposals_and_history_return_sorted_lists(tmp_path: Path) -> None:
    cfg = _make_agent(tmp_path)

    # Empty when there's nothing.
    assert list_proposals(cfg, "alpha") == []
    assert list_history(cfg, "alpha", "summarize") == []

    # Stage two proposals.
    asyncio.run(
        HumanApprovalCurationStrategy().handle(
            _proposal(skill_id="alpha-skill", body="a"),
            CurationCtx(config=cfg),
        )
    )
    asyncio.run(
        HumanApprovalCurationStrategy().handle(
            _proposal(skill_id="zeta-skill", body="z"),
            CurationCtx(config=cfg),
        )
    )

    proposals = list_proposals(cfg, "alpha")
    assert [p.parent.name for p in proposals] == ["alpha-skill", "zeta-skill"]

    # Build two history snapshots.
    auto = AutoPromoteCurationStrategy()
    asyncio.run(auto.handle(_proposal(body="v0"), CurationCtx(config=cfg)))
    time.sleep(0.01)
    asyncio.run(auto.handle(_proposal(body="v1"), CurationCtx(config=cfg)))
    time.sleep(0.01)
    asyncio.run(auto.handle(_proposal(body="v2"), CurationCtx(config=cfg)))

    hist = list_history(cfg, "alpha", "summarize")
    assert len(hist) == 2
    # Oldest first → "v0" before "v1".
    assert "v0" in hist[0].read_text(encoding="utf-8")
    assert "v1" in hist[1].read_text(encoding="utf-8")


# ---------- revert_skill --------------------------------------------------


def test_revert_skill_restores_most_recent_snapshot(tmp_path: Path) -> None:
    cfg = _make_agent(tmp_path)
    auto = AutoPromoteCurationStrategy()
    asyncio.run(auto.handle(_proposal(body="rev v0"), CurationCtx(config=cfg)))
    time.sleep(0.01)
    asyncio.run(auto.handle(_proposal(body="rev v1"), CurationCtx(config=cfg)))
    time.sleep(0.01)
    asyncio.run(auto.handle(_proposal(body="rev v2 — live"), CurationCtx(config=cfg)))

    pre_history = list_history(cfg, "alpha", "summarize")
    pre_history_count = len(pre_history)

    outcome = revert_skill(cfg, "alpha", "summarize")
    assert outcome.accepted is True
    live_text = outcome.target_path.read_text(encoding="utf-8")
    # Most-recent pre-revert snapshot was "rev v1" (v2 was live).
    assert "rev v1" in live_text

    # The live "rev v2" content should have been snapshotted as part of the revert.
    post_history = list_history(cfg, "alpha", "summarize")
    assert len(post_history) == pre_history_count + 1
    newest = post_history[-1].read_text(encoding="utf-8")
    assert "rev v2" in newest


# ---------- SkillCurator --------------------------------------------------


def test_curator_default_drafter_returns_no_outcomes(tmp_path: Path) -> None:
    cfg = _make_agent(tmp_path)
    curator = SkillCurator(cfg)
    outcomes = asyncio.run(
        curator.observe(
            [EventRecord(kind="tool_call_ended", agent_id="alpha", payload={})]
        )
    )
    assert outcomes == []


def test_curator_custom_drafter_dispatches_each_proposal(tmp_path: Path) -> None:
    cfg = _make_agent(tmp_path)

    def drafter(_batch: list[EventRecord]) -> list[SkillProposal]:
        return [
            _proposal(skill_id="one", body="first proposal"),
            _proposal(skill_id="two", body="second proposal"),
        ]

    curator = SkillCurator(cfg, drafter=drafter)
    outcomes = asyncio.run(
        curator.observe(
            [EventRecord(kind="tool_call_ended", agent_id="alpha", payload={})]
        )
    )
    assert len(outcomes) == 2
    assert all(o.accepted for o in outcomes)
    # Second proposal's target_path should exist on disk.
    second_target = outcomes[1].target_path
    assert second_target is not None
    assert second_target.is_file()
    assert "second proposal" in second_target.read_text(encoding="utf-8")


def test_curator_swapping_strategy_changes_target_semantics(tmp_path: Path) -> None:
    cfg = _make_agent(tmp_path)

    def drafter(_batch: list[EventRecord]) -> list[SkillProposal]:
        return [_proposal(skill_id="swap", body="swappable")]

    # Start auto-promote → live SKILL.md.
    curator = SkillCurator(cfg, drafter=drafter)
    auto_outcomes = asyncio.run(curator.observe([]))
    assert len(auto_outcomes) == 1
    assert auto_outcomes[0].accepted is True
    live_path = cfg.paths.agent("alpha").skills_dir / "swap" / "SKILL.md"
    assert auto_outcomes[0].target_path == live_path
    assert live_path.is_file()

    # Now swap to human-approval and re-run; should land in .proposed/.
    curator.strategy = HumanApprovalCurationStrategy()
    human_outcomes = asyncio.run(curator.observe([]))
    assert len(human_outcomes) == 1
    assert human_outcomes[0].accepted is False
    proposed_path = (
        cfg.paths.agent("alpha").skills_dir / ".proposed" / "swap" / "SKILL.md"
    )
    assert human_outcomes[0].target_path == proposed_path
    assert proposed_path.is_file()
