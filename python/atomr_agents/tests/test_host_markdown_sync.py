"""Tests for :mod:`atomr_agents.agent_host.markdown_sync`.

Gating strategy
---------------

The native PyO3 extension may be missing (e.g. an arm64 wheel that
hasn't been built, or a Python ABI mismatch). We probe it once at module
import — wrapping the probe in ``BaseException`` because pyo3 panics
during ``import`` aren't ``Exception`` subclasses and would otherwise
crash collection.

If ``_native`` *is* importable, every native-touching test runs. The
single "no native" test runs only when ``_native`` is **not**
importable, and asserts that ``AgentHostError`` is raised cleanly.

We drive async functions via :func:`asyncio.run` so the suite works
whether or not ``pytest-asyncio`` is configured.
"""

from __future__ import annotations

import asyncio
import shutil
import subprocess
import sys
import textwrap
from pathlib import Path

import pytest

# ---------- native availability probe ----------------------------------------
#
# Some PyO3 builds segfault during async ``store.put`` on a Python ABI they
# weren't built against (we've seen this on cpython 3.14 with a wheel
# compiled for an earlier ABI). A `try/except BaseException` cannot catch a
# C-level segfault, so we sniff for it in a short-lived subprocess. If the
# subprocess exits cleanly we trust the native ext on this interpreter.

_PROBE_SCRIPT = textwrap.dedent(
    """
    import asyncio
    from atomr_agents import _native
    if _native is None:
        raise SystemExit(2)

    async def main():
        store = _native.memory.in_memory_store()
        ns = _native.core.MemoryNamespace.agent("probe")
        item = _native.core.MemoryItem(
            id="probe:1",
            kind=_native.core.MemoryKind.semantic(),
            namespace=ns,
            payload={"text": "probe"},
            tags=["probe"],
        )
        await store.put(item)
        await store.list(ns, 10)

    asyncio.run(main())
    """
).strip()


def _native_works() -> bool:
    try:
        from atomr_agents import _native  # noqa: F401
    except BaseException:
        return False
    try:
        result = subprocess.run(
            [sys.executable, "-c", _PROBE_SCRIPT],
            capture_output=True,
            timeout=20,
        )
    except (subprocess.TimeoutExpired, OSError):
        return False
    return result.returncode == 0


_native_ok = _native_works()

# Re-import after the probe (caches the module attribute for later use).
try:
    from atomr_agents import _native  # type: ignore[attr-defined]
except BaseException:  # pragma: no cover
    _native = None  # type: ignore[assignment]

requires_native = pytest.mark.skipif(
    not _native_ok,
    reason="atomr_agents._native unavailable or unstable on this interpreter",
)

# ---------- shared imports / fixture path ------------------------------------

FIXTURE_HOST = (
    Path(__file__).parent / "fixtures" / "agent_host"
).resolve()
AGENT_ID = "alpha"


def _load_alpha():
    """Load the fixture agent. Imported lazily so the no-native test path
    still works when these modules fail to import their native deps."""
    from atomr_agents.agent_host.config import HostConfig
    from atomr_agents.agent_host.loader import AgentLoader

    config = HostConfig.load(FIXTURE_HOST)
    return config, AgentLoader(config).load(AGENT_ID)


def _new_store():
    return _native.memory.in_memory_store()


def _run(coro):
    return asyncio.run(coro)


# ---------- tests ------------------------------------------------------------


@requires_native
def test_sync_memory_facts_puts_expected_count_and_ids():
    from atomr_agents.agent_host.markdown_sync import sync_memory_facts

    _, loaded = _load_alpha()
    expected = list(loaded.memory_facts)
    assert len(expected) >= 1, "fixture should provide at least one MEMORY.md fact"

    store = _new_store()
    items = _run(sync_memory_facts(loaded, store))

    assert len(items) == len(expected)
    assert [it.id for it in items] == [f"memory_md:{i}" for i in range(1, len(expected) + 1)]
    assert all("memory_md" in (it.tags or []) for it in items)
    assert [it.payload["text"] for it in items] == expected


@requires_native
def test_sync_user_facts_puts_expected_count():
    from atomr_agents.agent_host.markdown_sync import (
        _bullet_lines,
        sync_user_facts,
    )

    _, loaded = _load_alpha()
    expected = _bullet_lines(loaded.user_profile)
    assert len(expected) >= 1, "fixture should provide at least one USER.md bullet"

    store = _new_store()
    items = _run(sync_user_facts(loaded, store))

    assert len(items) == len(expected)
    assert [it.id for it in items] == [f"user_md:{i}" for i in range(1, len(expected) + 1)]
    assert all("user_md" in (it.tags or []) for it in items)
    assert [it.payload["text"] for it in items] == expected


@requires_native
def test_sync_all_returns_correct_counts_dict():
    from atomr_agents.agent_host.markdown_sync import (
        _bullet_lines,
        sync_all,
    )

    _, loaded = _load_alpha()
    expected_mem = len(loaded.memory_facts)
    expected_usr = len(_bullet_lines(loaded.user_profile))

    store = _new_store()
    counts = _run(sync_all(loaded, store))

    assert counts == {"memory_md": expected_mem, "user_md": expected_usr}


@requires_native
def test_list_memory_facts_round_trips():
    from atomr_agents.agent_host.markdown_sync import (
        list_memory_facts,
        sync_all,
    )

    _, loaded = _load_alpha()
    expected = list(loaded.memory_facts)

    store = _new_store()
    _run(sync_all(loaded, store))
    listed = _run(list_memory_facts(loaded, store))

    assert all("memory_md" in (it.tags or []) for it in listed)
    assert sorted(it.payload["text"] for it in listed) == sorted(expected)
    # ensure USER.md items did NOT leak into memory_md tagged list
    assert all("user_md" not in (it.tags or []) for it in listed)


@requires_native
def test_list_user_facts_round_trips():
    from atomr_agents.agent_host.markdown_sync import (
        _bullet_lines,
        list_user_facts,
        sync_all,
    )

    _, loaded = _load_alpha()
    expected = _bullet_lines(loaded.user_profile)

    store = _new_store()
    _run(sync_all(loaded, store))
    listed = _run(list_user_facts(loaded, store))

    assert all("user_md" in (it.tags or []) for it in listed)
    assert sorted(it.payload["text"] for it in listed) == sorted(expected)
    assert all("memory_md" not in (it.tags or []) for it in listed)


@requires_native
def test_reload_agent_picks_up_on_disk_edit(tmp_path: Path):
    """Mutate MEMORY.md in a tmp copy of the fixture host and verify the
    reloaded agent + fresh sync now contains the new fact."""
    from atomr_agents.agent_host.config import HostConfig
    from atomr_agents.agent_host.loader import AgentLoader
    from atomr_agents.agent_host.markdown_sync import (
        list_memory_facts,
        reload_agent,
        sync_memory_facts,
    )

    # Copy fixture into tmp_path so we can safely mutate MEMORY.md.
    host_root = tmp_path / "host"
    shutil.copytree(FIXTURE_HOST, host_root)
    config = HostConfig.load(host_root)
    loader = AgentLoader(config)
    loaded = loader.load(AGENT_ID)
    baseline = len(loaded.memory_facts)

    store = _new_store()
    _run(sync_memory_facts(loaded, store))

    # Append a new bullet to MEMORY.md.
    mem_md = host_root / "agents" / AGENT_ID / "MEMORY.md"
    new_fact = "Agents prefer terse responses with concrete file paths."
    mem_md.write_text(
        mem_md.read_text(encoding="utf-8").rstrip() + f"\n- {new_fact}\n",
        encoding="utf-8",
    )

    # Reload + resync, then list and confirm the new fact is present.
    reloaded = reload_agent(config, AGENT_ID)
    assert len(reloaded.memory_facts) == baseline + 1
    assert new_fact in reloaded.memory_facts

    _run(sync_memory_facts(reloaded, store))
    listed = _run(list_memory_facts(reloaded, store))
    texts = [it.payload["text"] for it in listed]
    assert new_fact in texts


@pytest.mark.skipif(
    _native is not None,
    reason="native available; the no-native path can't be exercised",
)
def test_sync_raises_agent_host_error_without_native():
    """When ``_native`` is missing, calling any sync function should fail
    fast with :class:`AgentHostError` rather than an opaque
    ``AttributeError``."""
    from atomr_agents.agent_host.errors import AgentHostError
    from atomr_agents.agent_host.markdown_sync import sync_memory_facts

    # Use a tiny shim that mimics LoadedAgent's surface so we get past
    # attribute access before the native gate trips.
    class _StubSpec:
        id = AGENT_ID

    class _StubLoaded:
        spec = _StubSpec()
        memory_facts: list[str] = ["x"]
        user_profile = ""

    with pytest.raises(AgentHostError):
        asyncio.run(sync_memory_facts(_StubLoaded(), object()))
