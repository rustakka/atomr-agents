"""Tests for ``atomr_agents.agent_host.hooks`` — M5 hook substrate.

These tests are pure-Python and do not touch the PyO3 native extension.
"""

from __future__ import annotations

import asyncio
import json
import time
from pathlib import Path

import pytest

from atomr_agents.agent_host import AgentLoader, HostConfig
from atomr_agents.agent_host.hooks import (
    HookDispatcher,
    HookRegistry,
    HookResult,
    default_hook_resolver,
    matches,
    record_to_jsonl,
    redact_secrets,
)
from atomr_agents.agent_host.loader import HookDefinition


FIXTURE_ROOT = Path(__file__).parent / "fixtures" / "agent_host"


# ---------- helpers ---------------------------------------------------------


def _mk_hook(
    *,
    event: str = "on_tool_call",
    match: dict | None = None,
    call: dict | None = None,
    when: str = "pre",
    budget: dict | None = None,
) -> HookDefinition:
    return HookDefinition(
        event=event,
        match=match or {},
        call=call or {},
        when=when,
        budget=budget or {},
        source_path=None,
    )


# ---------- 1. matches() ----------------------------------------------------


def test_matches_empty_match_always_matches() -> None:
    hook = _mk_hook(match={})
    assert matches(hook, {"tool": "anything"}) is True
    assert matches(hook, {}) is True


def test_matches_single_key_match() -> None:
    hook = _mk_hook(match={"tool": "shell.exec"})
    assert matches(hook, {"tool": "shell.exec", "extra": 1}) is True


def test_matches_key_absent_in_payload() -> None:
    hook = _mk_hook(match={"tool": "shell.exec"})
    assert matches(hook, {"other": "value"}) is False


def test_matches_value_mismatch() -> None:
    hook = _mk_hook(match={"tool": "shell.exec"})
    assert matches(hook, {"tool": "fs.read"}) is False


# ---------- 2. HookRegistry -------------------------------------------------


def test_registry_register_returns_stable_id_and_counts() -> None:
    reg = HookRegistry()
    h1 = _mk_hook(event="evt")
    h2 = _mk_hook(event="evt")
    h3 = _mk_hook(event="other")

    async def impl(payload: dict, ctx: dict) -> None:
        return None

    id1 = reg.register(h1, impl)
    id2 = reg.register(h2, impl)
    id3 = reg.register(h3, impl)
    assert id1 == "evt#0"
    assert id2 == "evt#1"
    assert id3 == "other#0"
    assert len(reg) == 3


def test_registry_register_definitions_skips_when_resolver_returns_none() -> None:
    reg = HookRegistry()
    hooks = [
        _mk_hook(event="a", call={"kind": "builtin", "id": "redact_secrets"}),
        _mk_hook(event="b", call={"kind": "builtin", "id": "unknown"}),
    ]

    def resolver(defn: HookDefinition):
        if defn.call.get("id") == "redact_secrets":
            return redact_secrets()
        return None

    n = reg.register_definitions(hooks, resolver)
    assert n == 1
    assert len(reg) == 1
    # The skipped one isn't present.
    assert reg.hooks_for("b") == []


def test_registry_hooks_for_filters_by_event_and_when() -> None:
    reg = HookRegistry()

    async def impl(payload: dict, ctx: dict) -> None:
        return None

    pre_hook = _mk_hook(event="evt", when="pre")
    post_hook = _mk_hook(event="evt", when="post")
    both_hook = _mk_hook(event="evt", when="both")
    other_hook = _mk_hook(event="other", when="pre")
    reg.register(pre_hook, impl)
    reg.register(post_hook, impl)
    reg.register(both_hook, impl)
    reg.register(other_hook, impl)

    # By event
    all_evt = reg.hooks_for("evt")
    assert len(all_evt) == 3
    # By when=pre — pre + both
    pre = reg.hooks_for("evt", when="pre")
    pre_ids = {entry[0] for entry in pre}
    assert pre_ids == {"evt#0", "evt#2"}
    # By when=post — post + both
    post = reg.hooks_for("evt", when="post")
    post_ids = {entry[0] for entry in post}
    assert post_ids == {"evt#1", "evt#2"}
    # Different event
    other = reg.hooks_for("other")
    assert len(other) == 1


# ---------- 3. HookDispatcher baseline --------------------------------------


def test_dispatch_baseline_returns_ok_result() -> None:
    reg = HookRegistry()
    seen: list[dict] = []

    async def impl(payload: dict, ctx: dict) -> dict:
        seen.append(payload)
        return {"echo": payload.get("text")}

    hook = _mk_hook(event="on_tool_call", match={"tool": "shell.exec"})
    reg.register(hook, impl)

    dispatcher = HookDispatcher(reg)
    results = asyncio.run(
        dispatcher.dispatch(
            "on_tool_call", {"tool": "shell.exec", "text": "hi"}
        )
    )
    assert len(results) == 1
    r = results[0]
    assert isinstance(r, HookResult)
    assert r.ok is True
    assert r.hook_id == "on_tool_call#0"
    assert r.event == "on_tool_call"
    assert r.duration_ms >= 0.0
    assert r.output == {"echo": "hi"}
    assert seen == [{"tool": "shell.exec", "text": "hi"}]


def test_dispatch_skips_non_matching_hooks() -> None:
    reg = HookRegistry()

    async def impl(payload: dict, ctx: dict) -> dict:
        return {"ran": True}

    reg.register(_mk_hook(event="evt", match={"tool": "a"}), impl)
    reg.register(_mk_hook(event="evt", match={"tool": "b"}), impl)
    dispatcher = HookDispatcher(reg)
    results = asyncio.run(dispatcher.dispatch("evt", {"tool": "a"}))
    assert len(results) == 1
    assert results[0].hook_id == "evt#0"


# ---------- 4. HookDispatcher parallelism -----------------------------------


def test_dispatch_runs_hooks_in_parallel() -> None:
    reg = HookRegistry()

    def slow_sync(payload: dict, ctx: dict) -> dict:
        time.sleep(0.1)
        return {"slept": True}

    for _ in range(3):
        reg.register(_mk_hook(event="evt"), slow_sync)

    dispatcher = HookDispatcher(reg)
    start = time.perf_counter()
    results = asyncio.run(dispatcher.dispatch("evt", {}))
    elapsed = time.perf_counter() - start
    assert len(results) == 3
    assert all(r.ok for r in results)
    # Serial would be ~0.3s; parallel should be well under 0.25s.
    assert elapsed < 0.25, f"expected parallel dispatch, took {elapsed:.3f}s"


# ---------- 5. HookDispatcher timeout ---------------------------------------


def test_dispatch_timeout_produces_failure_result() -> None:
    reg = HookRegistry()

    async def slow(payload: dict, ctx: dict) -> dict:
        await asyncio.sleep(0.2)
        return {}

    reg.register(_mk_hook(event="evt", budget={"ms": 50}), slow)
    dispatcher = HookDispatcher(reg)
    results = asyncio.run(dispatcher.dispatch("evt", {}))
    assert len(results) == 1
    r = results[0]
    assert r.ok is False
    assert r.error is not None
    assert "timeout" in r.error.lower()
    assert r.output is None


# ---------- 6. HookDispatcher exception capture -----------------------------


def test_dispatch_captures_exceptions() -> None:
    reg = HookRegistry()

    async def boom(payload: dict, ctx: dict) -> dict:
        raise ValueError("kaboom")

    reg.register(_mk_hook(event="evt"), boom)
    dispatcher = HookDispatcher(reg)
    results = asyncio.run(dispatcher.dispatch("evt", {}))
    assert len(results) == 1
    r = results[0]
    assert r.ok is False
    assert r.error is not None
    assert "ValueError" in r.error
    assert "kaboom" in r.error


# ---------- 7. redact_secrets -----------------------------------------------


def test_redact_secrets_redacts_recognized_pattern() -> None:
    impl = redact_secrets()
    payload = {"text": "my api_key = sk-abc1234567890123456789012345"}
    out = asyncio.run(impl(payload, {}))
    assert "[REDACTED]" in out["text"]
    assert "sk-abc1234567890123456789012345" not in out["text"]
    # Caller's payload is not mutated.
    assert payload["text"] == "my api_key = sk-abc1234567890123456789012345"


def test_redact_secrets_passes_through_clean_text() -> None:
    impl = redact_secrets()
    payload = {"text": "nothing to redact here"}
    out = asyncio.run(impl(payload, {}))
    assert out["text"] == "nothing to redact here"


def test_redact_secrets_handles_missing_field() -> None:
    impl = redact_secrets()
    payload = {"other": 1}
    out = asyncio.run(impl(payload, {}))
    assert out == {"other": 1}


# ---------- 8. record_to_jsonl ----------------------------------------------


def test_record_to_jsonl_appends_lines(tmp_path: Path) -> None:
    target = tmp_path / "nested" / "events.jsonl"
    impl = record_to_jsonl(target)
    asyncio.run(impl({"text": "first"}, {"event": "on_tool_call"}))
    asyncio.run(impl({"text": "second"}, {"event": "on_tool_call"}))

    assert target.is_file()
    lines = target.read_text(encoding="utf-8").strip().splitlines()
    assert len(lines) == 2
    records = [json.loads(line) for line in lines]
    for rec in records:
        assert rec["event"] == "on_tool_call"
        assert "payload" in rec
        assert "ts_ms" in rec
    assert records[0]["payload"] == {"text": "first"}
    assert records[1]["payload"] == {"text": "second"}


# ---------- 9. default_hook_resolver ----------------------------------------


def test_default_resolver_binds_builtins(tmp_path: Path) -> None:
    resolver = default_hook_resolver(jsonl_path=tmp_path / "events.jsonl")
    redact_defn = _mk_hook(call={"kind": "builtin", "id": "redact_secrets"})
    jsonl_defn = _mk_hook(call={"kind": "builtin", "id": "record_to_jsonl"})
    unknown_defn = _mk_hook(call={"kind": "builtin", "id": "nope"})

    redact_impl = resolver(redact_defn)
    jsonl_impl = resolver(jsonl_defn)
    none_impl = resolver(unknown_defn)
    assert redact_impl is not None
    assert jsonl_impl is not None
    assert none_impl is None

    # Bound redact actually redacts.
    out = asyncio.run(redact_impl({"text": "AKIAABCDEFGHIJKLMNOP"}, {}))
    assert "[REDACTED]" in out["text"]


def test_default_resolver_returns_none_when_jsonl_path_missing() -> None:
    resolver = default_hook_resolver()  # no jsonl_path supplied
    defn = _mk_hook(call={"kind": "builtin", "id": "record_to_jsonl"})
    assert resolver(defn) is None


# ---------- 10. End-to-end via fixture --------------------------------------


def test_end_to_end_with_alpha_fixture() -> None:
    defn = AgentLoader(HostConfig.load(FIXTURE_ROOT)).parse("alpha")
    assert defn.hooks, "expected the alpha fixture to expose at least one hook"
    reg = HookRegistry()
    count = reg.register_definitions(defn.hooks, default_hook_resolver())
    assert count >= 1
    dispatcher = HookDispatcher(reg)
    payload = {
        "tool": "shell.exec",
        "text": "api_key=sk-abcdef1234567890123456789012",
    }
    results = asyncio.run(
        dispatcher.dispatch("on_tool_call", payload, when="pre")
    )
    assert any(r.ok for r in results)
    redacted = [r for r in results if r.ok and isinstance(r.output, dict)]
    assert redacted, "expected at least one hook to return the scrubbed payload"
    assert "[REDACTED]" in redacted[0].output["text"]
    # Caller's payload is unchanged.
    assert "sk-abcdef1234567890123456789012" in payload["text"]
