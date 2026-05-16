"""MarkdownMemorySync — thin facade over ``_native.host``.

Pushes bulleted facts from MEMORY.md / USER.md into a native
:class:`MemoryStore`, and reloads an agent's directory after on-disk
edits.

Bullet extraction is delegated to ``_native.host.split_bullets`` so
behavior stays in lock-step with the Rust loader.
"""

from __future__ import annotations

from typing import Any

from atomr_agents._native import host as _h

from .config import HostConfig
from .errors import AgentHostError
from .loader import AgentLoader, LoadedAgent

try:  # native extension is optional at import time
    from atomr_agents import _native as _native_pkg

    _native: Any | None = _native_pkg
except ImportError:  # pragma: no cover - covered by the no-native test
    _native = None

__all__ = [
    "list_memory_facts",
    "list_user_facts",
    "reload_agent",
    "sync_all",
    "sync_memory_facts",
    "sync_user_facts",
]


def _bullet_lines(body: str) -> list[str]:
    """Extract bulleted lines from a MEMORY/USER body via the native splitter."""
    return list(_h.split_bullets(body or ""))


def _require_native() -> Any:
    if _native is None:
        raise AgentHostError(
            "atomr_agents._native is not built — run `maturin develop` in the "
            "repo root before calling MarkdownMemorySync (sync_memory_facts/"
            "sync_user_facts/sync_all/list_memory_facts/list_user_facts)."
        )
    return _native


def _build_items(
    facts: list[str],
    *,
    agent_id: str,
    id_prefix: str,
    tag: str,
) -> list[Any]:
    native = _require_native()
    ns = native.core.MemoryNamespace.agent(agent_id)
    kind = native.core.MemoryKind.semantic()
    return [
        native.core.MemoryItem(
            id=f"{id_prefix}:{i}",
            kind=kind,
            namespace=ns,
            payload={"text": fact},
            tags=[tag],
        )
        for i, fact in enumerate(facts, start=1)
    ]


async def sync_memory_facts(loaded: LoadedAgent, store: Any) -> list[Any]:
    """Upsert each fact from ``MEMORY.md`` as a :class:`MemoryItem`."""
    items = _build_items(
        list(loaded.memory_facts),
        agent_id=loaded.spec.id,
        id_prefix="memory_md",
        tag="memory_md",
    )
    for item in items:
        await store.put(item)
    return items


async def sync_user_facts(loaded: LoadedAgent, store: Any) -> list[Any]:
    """Upsert each bulleted line from ``USER.md`` as a :class:`MemoryItem`."""
    items = _build_items(
        _bullet_lines(loaded.user_profile),
        agent_id=loaded.spec.id,
        id_prefix="user_md",
        tag="user_md",
    )
    for item in items:
        await store.put(item)
    return items


async def sync_all(loaded: LoadedAgent, store: Any) -> dict[str, int]:
    """Sync both MEMORY.md and USER.md facts. Returns count per source."""
    mem = await sync_memory_facts(loaded, store)
    usr = await sync_user_facts(loaded, store)
    return {"memory_md": len(mem), "user_md": len(usr)}


async def _list_by_tag(loaded: LoadedAgent, store: Any, tag: str) -> list[Any]:
    native = _require_native()
    ns = native.core.MemoryNamespace.agent(loaded.spec.id)
    items = await store.list(ns, 10_000)
    return [it for it in items if tag in (it.tags or [])]


async def list_memory_facts(loaded: LoadedAgent, store: Any) -> list[Any]:
    """Return items in the agent's namespace tagged ``memory_md``."""
    return await _list_by_tag(loaded, store, "memory_md")


async def list_user_facts(loaded: LoadedAgent, store: Any) -> list[Any]:
    """Return items in the agent's namespace tagged ``user_md``."""
    return await _list_by_tag(loaded, store, "user_md")


def reload_agent(config: HostConfig, agent_id: str) -> LoadedAgent:
    """Re-parse and re-materialize the agent. Used after on-disk edits."""
    return AgentLoader(config).load(agent_id)
