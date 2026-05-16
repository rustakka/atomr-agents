"""Tests for the M7 multi-channel gateway + AGENTS.md routing.

The pure-Python parser / router builder is exercised unconditionally;
the :class:`Gateway` tests that touch the native ``ChannelHarness``
are gated by a probe so they skip cleanly on the Python 3.14 wheel
(where harness construction currently panics).
"""

from __future__ import annotations

import asyncio
from pathlib import Path

import pytest

pytest.importorskip("yaml")

from atomr_agents.agent_host import HostConfig
from atomr_agents.agent_host.gateway import (
    AgentsRoutingRules,
    Gateway,
    build_router,
    load_agents_md,
    parse_agents_md,
)


FIXTURE_ROOT = Path(__file__).parent / "fixtures" / "agent_host"


def _probe_channel() -> bool:
    """Return True iff ``ChannelHarness()`` constructs without panicking."""
    try:
        from atomr_agents import _native

        _native.channel.ChannelHarness()
        return True
    except BaseException:
        return False


_channel_ok = _probe_channel()
requires_channel = pytest.mark.skipif(
    not _channel_ok, reason="channel harness unusable"
)


# ---------- parse_agents_md (native-free) ----------------------------------


def test_parse_agents_md_empty_input_returns_empty_rules() -> None:
    rules = parse_agents_md("")
    assert isinstance(rules, AgentsRoutingRules)
    assert rules.default_agent is None
    assert rules.channel_pins == {}
    assert rules.peer_pins == {}


def test_parse_agents_md_defaults_only() -> None:
    text = """# AGENTS.md

## Defaults
- Any unmatched message: default
"""
    rules = parse_agents_md(text)
    assert rules.default_agent == "default"
    assert rules.channel_pins == {}
    assert rules.peer_pins == {}


def test_parse_agents_md_defaults_alt_phrasings() -> None:
    text = """## Defaults
- => alpha
"""
    rules = parse_agents_md(text)
    assert rules.default_agent == "alpha"

    text2 = """## Defaults
- alpha
"""
    assert parse_agents_md(text2).default_agent == "alpha"

    text3 = """## Defaults
- → alpha
"""
    assert parse_agents_md(text3).default_agent == "alpha"


def test_parse_agents_md_channel_pins_mixed_arrows_and_colons() -> None:
    text = """## Channel pins
- discord:server-1 → ops-bot
- slack:team-eng: dev-bot
- cli:local -> debug-bot
"""
    rules = parse_agents_md(text)
    assert rules.channel_pins == {
        "discord:server-1": "ops-bot",
        "slack:team-eng": "dev-bot",
        "cli:local": "debug-bot",
    }
    assert rules.default_agent is None


def test_parse_agents_md_peer_pins_with_colon_channel_ids() -> None:
    text = """## Peer pins
- discord:server-1 @alerts → incident-bot
- slack:team-eng @oncall -> pager-bot
- cli:local user → alpha
"""
    rules = parse_agents_md(text)
    assert rules.peer_pins == {
        ("discord:server-1", "@alerts"): "incident-bot",
        ("slack:team-eng", "@oncall"): "pager-bot",
        ("cli:local", "user"): "alpha",
    }


def test_parse_agents_md_malformed_bullets_are_ignored() -> None:
    text = """## Defaults
- Any unmatched message: default

## Channel pins
- this is not a valid pin
-
- discord:server-1 → ops-bot
- no-arrow-no-colon

## Peer pins
- only-one-token →
- channel peer → agent
- gibberish without arrow
"""
    rules = parse_agents_md(text)
    assert rules.default_agent == "default"
    assert rules.channel_pins == {"discord:server-1": "ops-bot"}
    # Only the valid peer pin is kept.
    assert rules.peer_pins == {("channel", "peer"): "agent"}


def test_parse_agents_md_full_doc() -> None:
    text = """# AGENTS.md

Some prose. Will be ignored — no section header yet.

## Defaults
- Any unmatched message: default

## Channel pins
- discord:server-1 → ops-bot
- slack:team-eng: dev-bot

## Peer pins
- discord:server-1 @alerts → incident-bot

## Notes
- this section is not recognized and is ignored
"""
    rules = parse_agents_md(text)
    assert rules.default_agent == "default"
    assert rules.channel_pins == {
        "discord:server-1": "ops-bot",
        "slack:team-eng": "dev-bot",
    }
    assert rules.peer_pins == {("discord:server-1", "@alerts"): "incident-bot"}


def test_load_agents_md_missing_file_returns_empty(tmp_path: Path) -> None:
    from atomr_agents.agent_host.layout import HostPaths

    paths = HostPaths(root=tmp_path)
    assert not paths.agents_md.exists()
    rules = load_agents_md(paths)
    assert isinstance(rules, AgentsRoutingRules)
    assert rules.default_agent is None
    assert rules.channel_pins == {}
    assert rules.peer_pins == {}
    # source_path is still populated so callers can inspect what was looked up.
    assert rules.source_path == paths.agents_md


def test_load_agents_md_reads_existing_file(tmp_path: Path) -> None:
    from atomr_agents.agent_host.layout import HostPaths

    paths = HostPaths(root=tmp_path)
    paths.agents_md.write_text(
        """## Defaults
- Any unmatched message: bravo
""",
        encoding="utf-8",
    )
    rules = load_agents_md(paths)
    assert rules.default_agent == "bravo"
    assert rules.source_path == paths.agents_md


# ---------- build_router (native-free) -------------------------------------


def _empty_host_config(tmp_path: Path, *, default_agent: str | None = None) -> HostConfig:
    """Build a minimal HostConfig rooted at ``tmp_path``."""
    from atomr_agents.agent_host.config import HostConfig as _HC
    from atomr_agents.agent_host.layout import HostPaths

    paths = HostPaths(root=tmp_path)
    return _HC(paths=paths, default_agent=default_agent)


def test_build_router_agents_md_default_beats_config(tmp_path: Path) -> None:
    cfg = _empty_host_config(tmp_path, default_agent="from-config")
    rules = AgentsRoutingRules(default_agent="from-agents-md")
    router = build_router(cfg, agents_md=rules)
    assert router.default_agent == "from-agents-md"


def test_build_router_falls_back_to_config_default(tmp_path: Path) -> None:
    cfg = _empty_host_config(tmp_path, default_agent="from-config")
    rules = AgentsRoutingRules()  # no default in AGENTS.md
    router = build_router(cfg, agents_md=rules)
    assert router.default_agent == "from-config"


def test_build_router_pins_flow_through(tmp_path: Path) -> None:
    cfg = _empty_host_config(tmp_path, default_agent="default")
    rules = AgentsRoutingRules(
        default_agent=None,
        channel_pins={"discord:server-1": "ops-bot"},
        peer_pins={("discord:server-1", "@alerts"): "incident-bot"},
    )
    router = build_router(cfg, agents_md=rules)
    assert router.default_agent == "default"
    assert router.route("discord:server-1", "anyone") == "ops-bot"
    assert router.route("discord:server-1", "@alerts") == "incident-bot"
    assert router.route("other-channel", "user") == "default"


def test_build_router_reads_disk_when_agents_md_arg_omitted(tmp_path: Path) -> None:
    from atomr_agents.agent_host.layout import HostPaths

    paths = HostPaths(root=tmp_path)
    paths.agents_md.write_text(
        """## Defaults
- Any unmatched message: from-disk

## Channel pins
- cli:test → alpha
""",
        encoding="utf-8",
    )
    cfg = _empty_host_config(tmp_path, default_agent="ignored")
    router = build_router(cfg)
    # AGENTS.md default wins over config default.
    assert router.default_agent == "from-disk"
    assert router.route("cli:test", "user") == "alpha"


# ---------- Gateway (native-required) --------------------------------------


@requires_channel
def test_gateway_session_for_returns_cached_session() -> None:
    cfg = HostConfig.load(FIXTURE_ROOT)
    gw = Gateway(cfg)

    async def go() -> None:
        try:
            s1 = await gw.session_for("cli:gw1", "alice")
            s2 = await gw.session_for("cli:gw1", "alice")
            assert s1 is s2
            s3 = await gw.session_for("cli:gw1", "bob")
            assert s3 is not s1
            assert sorted(gw.open_session_ids()) == [
                ("cli:gw1", "alice"),
                ("cli:gw1", "bob"),
            ]
        finally:
            await gw.close()

    asyncio.run(go())


@requires_channel
def test_gateway_send_round_trip_includes_persona() -> None:
    cfg = HostConfig.load(FIXTURE_ROOT)
    gw = Gateway(cfg)

    async def go() -> str:
        try:
            return await gw.send("cli:gw2", "alice", "hello")
        finally:
            await gw.close()

    reply = asyncio.run(go())
    # Persona identity (loaded from fixture SOUL.md) must appear.
    assert "alpha" in reply
    assert "pragmatic engineering" in reply
    assert "user: hello" in reply


@requires_channel
def test_gateway_two_keys_same_agent_distinct_sessions() -> None:
    cfg = HostConfig.load(FIXTURE_ROOT)
    gw = Gateway(cfg)

    async def go() -> tuple[str, str, list[tuple[str, str]]]:
        try:
            r_a = await gw.send("cli:room-a", "alice", "ping-a")
            r_b = await gw.send("cli:room-b", "bob", "ping-b")
            return r_a, r_b, sorted(gw.open_session_ids())
        finally:
            await gw.close()

    reply_a, reply_b, keys = asyncio.run(go())
    assert "user: ping-a" in reply_a
    assert "user: ping-b" in reply_b
    # Both keys are open in the gateway's session map.
    assert keys == [("cli:room-a", "alice"), ("cli:room-b", "bob")]


@requires_channel
def test_gateway_close_clears_session_map() -> None:
    cfg = HostConfig.load(FIXTURE_ROOT)
    gw = Gateway(cfg)

    async def go() -> None:
        await gw.session_for("cli:gw-close", "alice")
        await gw.session_for("cli:gw-close", "bob")
        assert len(gw.open_session_ids()) == 2
        await gw.close()
        assert gw.open_session_ids() == []

    asyncio.run(go())
