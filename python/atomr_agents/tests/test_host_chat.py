"""Tests for the M2 local CLI channel + AgentRouter.

These tests need the native PyO3 extension and a working tokio
runtime. The 3.14 wheel currently panics on ``ChannelHarness()``
construction (pre-existing FFI issue), so we gate by probing the
harness once and skipping cleanly if it can't be constructed.
"""

from __future__ import annotations

import asyncio
import io
import json
from pathlib import Path

import pytest

pytest.importorskip("yaml")

from atomr_agents.agent_host import (
    AgentLoader,
    AgentRouter,
    ChatSession,
    HostConfig,
    chat_repl,
    render_chat_preview,
    thread_log_path,
)
from atomr_agents.agent_host.errors import AgentHostError


FIXTURE_ROOT = Path(__file__).parent / "fixtures" / "agent_host"


def _probe_native_chat() -> bool:
    """Return True iff ``ChannelHarness()`` constructs without panicking.

    The 3.14 wheel currently panics in tokio runtime init; the probe is
    wrapped in ``BaseException`` (PanicException isn't always an
    ``Exception`` subclass under PyO3 0.22).
    """
    try:
        from atomr_agents import _native as _native_pkg

        _native_pkg.channel.ChannelHarness()
        return True
    except BaseException:
        return False


_native_chat_available = _probe_native_chat()


requires_native_chat = pytest.mark.skipif(
    not _native_chat_available,
    reason=(
        "atomr_agents._native channel harness not usable in this interpreter "
        "(known issue under Python 3.14 wheels)"
    ),
)


# ---------- AgentRouter (pure-Python, always runs) -------------------------


def test_router_default_only() -> None:
    r = AgentRouter(default_agent="alpha")
    assert r.route("cli:local", "user") == "alpha"


def test_router_no_default_raises() -> None:
    r = AgentRouter()
    with pytest.raises(AgentHostError):
        r.route("cli:local", "user")


def test_router_channel_pin() -> None:
    r = AgentRouter(default_agent="alpha")
    r.pin_channel("discord:server-1", "beta")
    assert r.route("discord:server-1", "anyone") == "beta"
    assert r.route("cli:local", "user") == "alpha"


def test_router_peer_pin_beats_channel_pin() -> None:
    r = AgentRouter(default_agent="alpha")
    r.pin_channel("discord:server-1", "beta")
    r.pin_peer("discord:server-1", "carol", "gamma")
    assert r.route("discord:server-1", "carol") == "gamma"
    assert r.route("discord:server-1", "dave") == "beta"


# ---------- render_chat_preview (native-gated only because it needs `loaded.persona`) ------


@pytest.fixture
def loaded_alpha():
    cfg = HostConfig.load(FIXTURE_ROOT)
    return AgentLoader(cfg).load("alpha")


@pytest.fixture
def loaded_alpha_or_skip():
    if not _native_chat_available:
        pytest.skip("native chat path unavailable")
    cfg = HostConfig.load(FIXTURE_ROOT)
    return AgentLoader(cfg).load("alpha")


def test_render_chat_preview_includes_persona_and_counts(loaded_alpha) -> None:
    out = render_chat_preview(loaded_alpha, "hello")
    assert "alpha" in out
    assert "pragmatic engineering" in out
    assert "user: hello" in out
    # rules / memory / skills counts present
    assert "rules: 3" in out
    assert "memory facts: 2" in out
    assert "skills: 1" in out


def test_render_chat_preview_with_empty_persona(loaded_alpha) -> None:
    # Mutate a copy with no persona to exercise the fallback branch.
    from dataclasses import replace

    bare = replace(loaded_alpha, persona=None)
    out = render_chat_preview(bare, "yo")
    assert "(no persona)" in out


def test_thread_log_path_sanitizes_separators(loaded_alpha) -> None:
    p = thread_log_path(loaded_alpha, "cli:local", "cli:local#alice/x")
    assert ":" not in p.name
    assert "/" not in p.name
    assert p.parent.name == "cli__local"


# ---------- ChatSession round-trip (native-required) ------------------------


@requires_native_chat
def test_chat_session_round_trip(loaded_alpha_or_skip, tmp_path: Path) -> None:
    """Single send → reply → close, with the JSONL log persisted."""

    loaded = loaded_alpha_or_skip

    async def go() -> str:
        session = ChatSession(
            loaded=loaded, channel_id="cli:test", peer="probe", persist=True
        )
        await session.open()
        try:
            return await session.send("ping")
        finally:
            await session.close()

    reply = asyncio.run(go())
    assert "alpha" in reply
    assert "user: ping" in reply

    log_path = thread_log_path(loaded, "cli:test", "cli:test#probe")
    assert log_path.is_file(), f"JSONL log not written at {log_path}"
    entries = [json.loads(line) for line in log_path.read_text().splitlines() if line.strip()]
    kinds = [e["kind"] for e in entries]
    assert kinds == ["thread_opened", "user_message", "agent_reply", "thread_closed"]

    # Clean up the persisted log so re-running tests is hermetic.
    log_path.unlink(missing_ok=True)


@requires_native_chat
def test_chat_session_no_persist(loaded_alpha_or_skip) -> None:
    loaded = loaded_alpha_or_skip

    async def go() -> None:
        session = ChatSession(
            loaded=loaded, channel_id="cli:np", peer="probe", persist=False
        )
        await session.open()
        reply = await session.send("noop")
        assert "user: noop" in reply
        await session.close()

    asyncio.run(go())
    assert not thread_log_path(loaded, "cli:np", "cli:np#probe").exists()


@requires_native_chat
def test_chat_session_send_before_open_raises(loaded_alpha_or_skip) -> None:
    loaded = loaded_alpha_or_skip
    session = ChatSession(loaded=loaded, persist=False)
    with pytest.raises(AgentHostError):
        asyncio.run(session.send("nope"))


@requires_native_chat
def test_chat_repl_piped_stdin(loaded_alpha_or_skip) -> None:
    """The interactive REPL drains piped lines and exits on /quit."""

    loaded = loaded_alpha_or_skip
    stdin = io.StringIO("hi\n/quit\n")
    stdout = io.StringIO()
    chat_repl(loaded, channel_id="cli:repl", peer="me", persist=False, in_stream=stdin, out_stream=stdout)
    transcript = stdout.getvalue()
    assert "atomr-host chat" in transcript
    assert "user: hi" in transcript
