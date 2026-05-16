"""Tests for ``atomr_agents.agent_host.scheduler`` — M6 scheduler.

These tests are pure-Python and do not touch the PyO3 native extension.
They drive the async API via :func:`asyncio.run` — ``pytest-asyncio`` is
not relied upon here so the file is robust across pytest configs.
"""

from __future__ import annotations

import asyncio
import time
from pathlib import Path

import pytest

from atomr_agents.agent_host import HostConfig
from atomr_agents.agent_host.errors import AgentSpecError
from atomr_agents.agent_host.layout import HostPaths
from atomr_agents.agent_host.scheduler import (
    CronEntry,
    CronFireResult,
    Scheduler,
    default_cron_resolver,
    load_crons,
    parse_expression,
    scaffold_cron,
)


# ---------- helpers ---------------------------------------------------------


class _ManualClock:
    """Manually-advanced monotonic clock used to drive Scheduler deterministically.

    The current time is held in a list cell so :py:meth:`current` is a
    plain callable that the scheduler can store. Tests advance time by
    setting :py:attr:`now` directly.
    """

    def __init__(self, start: float = 0.0) -> None:
        self._t = [float(start)]

    @property
    def now(self) -> float:
        return self._t[0]

    @now.setter
    def now(self, value: float) -> None:
        self._t[0] = float(value)

    def advance(self, delta: float) -> None:
        self._t[0] += float(delta)

    def __call__(self) -> float:
        return self._t[0]


def _mk_config(root: Path) -> HostConfig:
    """Build a minimal :class:`HostConfig` rooted at ``root``."""
    return HostConfig(paths=HostPaths(root=root))


# ---------- 1. parse_expression --------------------------------------------


def test_parse_expression_seconds() -> None:
    assert parse_expression("every:30s") == 30.0


def test_parse_expression_minutes() -> None:
    assert parse_expression("every:5m") == 300.0


def test_parse_expression_hours() -> None:
    assert parse_expression("every:2h") == 2.0 * 3600.0


def test_parse_expression_days() -> None:
    assert parse_expression("every:1d") == 86400.0


def test_parse_expression_strips_whitespace() -> None:
    assert parse_expression("  every:10s  ") == 10.0


def test_parse_expression_rejects_zero() -> None:
    with pytest.raises(AgentSpecError):
        parse_expression("every:0s")


def test_parse_expression_rejects_negative() -> None:
    with pytest.raises(AgentSpecError):
        parse_expression("every:-5m")


def test_parse_expression_rejects_unknown_unit() -> None:
    with pytest.raises(AgentSpecError):
        parse_expression("every:10x")


def test_parse_expression_rejects_full_crontab() -> None:
    # M6 is explicit that full crontab strings are out of scope.
    with pytest.raises(AgentSpecError):
        parse_expression("* * * * *")


def test_parse_expression_rejects_empty_interval() -> None:
    with pytest.raises(AgentSpecError):
        parse_expression("every:")


def test_parse_expression_rejects_non_string() -> None:
    with pytest.raises(AgentSpecError):
        parse_expression(30)  # type: ignore[arg-type]


# ---------- 2. load_crons ---------------------------------------------------


def test_load_crons_reads_yaml_files(tmp_path: Path) -> None:
    cfg = _mk_config(tmp_path)
    cfg.paths.ensure()
    (cfg.paths.crons_dir / "heartbeat.yaml").write_text(
        "expression: every:30s\n"
        "call:\n  kind: builtin\n  id: noop\n"
        "input: {}\n",
        encoding="utf-8",
    )
    (cfg.paths.crons_dir / "daily.yaml").write_text(
        "expression: every:1d\n"
        "call:\n  kind: builtin\n  id: noop\n"
        "input:\n  greeting: hi\n",
        encoding="utf-8",
    )

    entries = load_crons(cfg)
    ids = sorted(e.id for e in entries)
    assert ids == ["daily", "heartbeat"]
    by_id = {e.id: e for e in entries}
    assert by_id["heartbeat"].expression == "every:30s"
    assert by_id["daily"].expression == "every:1d"
    assert by_id["daily"].input == {"greeting": "hi"}
    assert by_id["heartbeat"].call == {"kind": "builtin", "id": "noop"}
    assert all(e.source_path is not None for e in entries)


def test_load_crons_skips_dotfiles(tmp_path: Path) -> None:
    cfg = _mk_config(tmp_path)
    cfg.paths.ensure()
    (cfg.paths.crons_dir / ".hidden.yaml").write_text(
        "expression: every:30s\ncall: {}\n", encoding="utf-8"
    )
    (cfg.paths.crons_dir / "real.yaml").write_text(
        "expression: every:1m\ncall: {}\n", encoding="utf-8"
    )
    entries = load_crons(cfg)
    assert [e.id for e in entries] == ["real"]


def test_load_crons_returns_empty_when_dir_missing(tmp_path: Path) -> None:
    cfg = _mk_config(tmp_path)
    # Do NOT call cfg.paths.ensure() — crons/ does not exist.
    assert load_crons(cfg) == []


def test_load_crons_validates_expression(tmp_path: Path) -> None:
    cfg = _mk_config(tmp_path)
    cfg.paths.ensure()
    (cfg.paths.crons_dir / "bad.yaml").write_text(
        "expression: every:0s\ncall: {}\n", encoding="utf-8"
    )
    with pytest.raises(AgentSpecError):
        load_crons(cfg)


# ---------- 3. Scheduler with manual clock ---------------------------------


def test_scheduler_register_seeds_first_fire_one_interval_out() -> None:
    clock = _ManualClock(start=0.0)
    sched = Scheduler(clock=clock)
    entry = CronEntry(id="hb", expression="every:1m", call={})

    async def _impl(input: dict, ctx: dict) -> dict:
        return {"ok": True}

    sched.register(entry, _impl)
    # First fire is exactly one interval after register() — clock was 0.
    assert sched.next_fire("hb") == pytest.approx(60.0)


def test_scheduler_fire_due_lifecycle_with_manual_clock() -> None:
    clock = _ManualClock(start=0.0)
    sched = Scheduler(clock=clock)
    entry = CronEntry(id="hb", expression="every:1m", call={})

    call_count = {"n": 0}

    async def _impl(input: dict, ctx: dict) -> dict:
        call_count["n"] += 1
        return {"call": call_count["n"]}

    sched.register(entry, _impl)

    # At t=0: nothing is due yet (next fire is at 60).
    results = asyncio.run(sched.fire_due())
    assert results == []
    assert call_count["n"] == 0

    # Advance to t=60.5 → entry fires; next fire is now ~120.5.
    clock.now = 60.5
    results = asyncio.run(sched.fire_due())
    assert len(results) == 1
    assert results[0].ok is True
    assert results[0].entry_id == "hb"
    assert results[0].output == {"call": 1}
    assert call_count["n"] == 1
    # post-fire clock is still 60.5, so the next fire is 60.5 + 60 = 120.5
    assert sched.next_fire("hb") == pytest.approx(120.5)

    # Advance to t=121 → fires again.
    clock.now = 121.0
    results = asyncio.run(sched.fire_due())
    assert len(results) == 1
    assert results[0].output == {"call": 2}
    assert call_count["n"] == 2
    assert sched.next_fire("hb") == pytest.approx(181.0)


def test_scheduler_sync_impl_is_auto_wrapped() -> None:
    clock = _ManualClock(start=0.0)
    sched = Scheduler(clock=clock)
    entry = CronEntry(id="hb", expression="every:1s", call={})

    def _impl(input: dict, ctx: dict) -> dict:
        return {"sync": True}

    sched.register(entry, _impl)
    clock.now = 2.0
    results = asyncio.run(sched.fire_due())
    assert len(results) == 1
    assert results[0].ok is True
    assert results[0].output == {"sync": True}


def test_scheduler_passes_ctx_through() -> None:
    clock = _ManualClock(start=0.0)
    sched = Scheduler(clock=clock)
    entry = CronEntry(id="hb", expression="every:1s", call={}, input={"x": 1})

    seen = {}

    async def _impl(input: dict, ctx: dict) -> dict:
        seen["input"] = input
        seen["ctx"] = ctx
        return {}

    sched.register(entry, _impl)
    clock.now = 2.0
    asyncio.run(sched.fire_due(ctx={"agent": "default"}))
    assert seen["input"] == {"x": 1}
    assert seen["ctx"] == {"agent": "default"}


# ---------- 4. Parallelism --------------------------------------------------


def test_scheduler_fire_due_runs_in_parallel() -> None:
    """Three impls each sleeping 0.05s must finish well under 0.15s wall-clock.

    Uses real time.monotonic so the sleep actually elapses.
    """
    sched = Scheduler()  # default real-monotonic clock

    async def _slow(input: dict, ctx: dict) -> dict:
        await asyncio.sleep(0.05)
        return {"ok": True}

    # Three entries due immediately: parse_expression('every:1s') seeds
    # next-fire to monotonic+1s, so we force them past-due via
    # schedule_after(0).
    for i in range(3):
        entry = CronEntry(id=f"slow-{i}", expression="every:1s", call={})
        sched.register(entry, _slow)
        sched.schedule_after(entry.id, -1.0)  # already past-due

    start = time.monotonic()
    results = asyncio.run(sched.fire_due())
    elapsed = time.monotonic() - start

    assert len(results) == 3
    assert all(r.ok for r in results)
    # Serial would be >= 0.15s; parallel should be comfortably under 0.12s.
    assert elapsed < 0.12, f"fire_due was not parallel: elapsed={elapsed:.3f}s"


# ---------- 5. Disabled entries --------------------------------------------


def test_scheduler_disabled_entry_never_fires() -> None:
    clock = _ManualClock(start=0.0)
    sched = Scheduler(clock=clock)
    entry = CronEntry(id="off", expression="every:1s", call={}, enabled=False)

    called = {"n": 0}

    async def _impl(input: dict, ctx: dict) -> dict:
        called["n"] += 1
        return {}

    sched.register(entry, _impl)
    # Advance well past the next-fire mark; the entry must still be skipped.
    clock.now = 1000.0
    results = asyncio.run(sched.fire_due())
    assert results == []
    assert called["n"] == 0


# ---------- 6. Exception capture --------------------------------------------


def test_scheduler_captures_exception_into_result() -> None:
    clock = _ManualClock(start=0.0)
    sched = Scheduler(clock=clock)
    entry = CronEntry(id="boom", expression="every:1s", call={})

    async def _impl(input: dict, ctx: dict) -> dict:
        raise RuntimeError("kaboom")

    sched.register(entry, _impl)
    clock.now = 2.0
    results = asyncio.run(sched.fire_due())
    assert len(results) == 1
    r = results[0]
    assert isinstance(r, CronFireResult)
    assert r.ok is False
    assert r.output is None
    assert r.error is not None
    assert "RuntimeError" in r.error
    assert "kaboom" in r.error
    # And the entry was rescheduled even though it failed.
    assert sched.next_fire("boom") is not None
    assert sched.next_fire("boom") > 2.0


# ---------- 7. scaffold_cron round-trips through load_crons ----------------


def test_scaffold_cron_round_trip(tmp_path: Path) -> None:
    cfg = _mk_config(tmp_path)
    cfg.paths.ensure()

    path = scaffold_cron(cfg, "heartbeat", when="every:30s")
    assert path.exists()
    assert path == cfg.paths.crons_dir / "heartbeat.yaml"

    entries = load_crons(cfg)
    assert len(entries) == 1
    entry = entries[0]
    assert entry.id == "heartbeat"
    assert entry.expression == "every:30s"
    assert entry.call == {"kind": "builtin", "id": "noop"}
    assert entry.input == {}
    assert entry.enabled is True


def test_scaffold_cron_is_idempotent(tmp_path: Path) -> None:
    cfg = _mk_config(tmp_path)
    cfg.paths.ensure()
    path = scaffold_cron(cfg, "hb", when="every:1m")
    original = path.read_text(encoding="utf-8")
    # Second call with a different `when` should NOT clobber the file.
    scaffold_cron(cfg, "hb", when="every:5m")
    assert path.read_text(encoding="utf-8") == original


def test_scaffold_cron_force_overwrites(tmp_path: Path) -> None:
    cfg = _mk_config(tmp_path)
    cfg.paths.ensure()
    path = scaffold_cron(cfg, "hb", when="every:1m")
    scaffold_cron(cfg, "hb", when="every:5m", force=True)
    body = path.read_text(encoding="utf-8")
    assert "every:5m" in body


def test_scaffold_cron_rejects_bad_when(tmp_path: Path) -> None:
    cfg = _mk_config(tmp_path)
    cfg.paths.ensure()
    with pytest.raises(AgentSpecError):
        scaffold_cron(cfg, "hb", when="not-a-cron")


# ---------- 8. default_cron_resolver ---------------------------------------


def test_default_cron_resolver_binds_builtin_noop() -> None:
    resolver = default_cron_resolver()
    entry = CronEntry(
        id="x", expression="every:1s", call={"kind": "builtin", "id": "noop"}
    )
    impl = resolver(entry)
    assert impl is not None
    out = asyncio.run(impl({"hello": "world"}, {}))
    # Built-in noop echoes its input.
    assert out == {"hello": "world"}


def test_default_cron_resolver_unknown_id_returns_none() -> None:
    resolver = default_cron_resolver()
    entry = CronEntry(
        id="x", expression="every:1s", call={"kind": "builtin", "id": "does-not-exist"}
    )
    assert resolver(entry) is None


def test_default_cron_resolver_non_builtin_kind_returns_none() -> None:
    resolver = default_cron_resolver()
    entry = CronEntry(
        id="x", expression="every:1s", call={"kind": "skill", "id": "noop"}
    )
    assert resolver(entry) is None


# ---------- bonus coverage: registering through the resolver ---------------


def test_default_resolver_drives_scheduler_end_to_end(tmp_path: Path) -> None:
    cfg = _mk_config(tmp_path)
    cfg.paths.ensure()
    scaffold_cron(cfg, "ping", when="every:1s", input={"echo": 7})

    clock = _ManualClock(start=0.0)
    sched = Scheduler(clock=clock)
    resolver = default_cron_resolver()

    for entry in load_crons(cfg):
        impl = resolver(entry)
        assert impl is not None
        sched.register(entry, impl)

    clock.now = 2.0
    results = asyncio.run(sched.fire_due())
    assert len(results) == 1
    assert results[0].ok is True
    assert results[0].output == {"echo": 7}
