"""Smoke tests for Phase 1.4 memory + embed bindings."""

from __future__ import annotations

import pytest

from atomr_agents import core as core_mod
from atomr_agents import embed as embed_mod
from atomr_agents import memory as memory_mod


@pytest.mark.asyncio
async def test_in_memory_store_put_list():
    store = memory_mod.in_memory_store()
    ns = core_mod.MemoryNamespace.agent("a-1")
    item = core_mod.MemoryItem(
        id="m1",
        kind=core_mod.MemoryKind.episodic(),
        namespace=ns,
        payload={"text": "hello"},
    )
    await store.put(item)
    items = await store.list(ns, 10)
    assert len(items) == 1
    assert items[0].id == "m1"


@pytest.mark.asyncio
async def test_in_memory_long_store_round_trip():
    long_store = memory_mod.in_memory_long_store()
    ns = memory_mod.Namespace.from_parts(["user", "alice"])
    await long_store.put(ns, "city", "Boston", None)
    found = await long_store.get(ns, "city")
    assert found is not None
    assert found.value == "Boston"


@pytest.mark.asyncio
async def test_mock_embedder_is_deterministic():
    e = embed_mod.mock_embedder(8)
    v1 = await e.embed("hello")
    v2 = await e.embed("hello")
    v3 = await e.embed("world")
    assert v1 == v2
    assert v1 != v3
    assert len(v1) == 8


@pytest.mark.asyncio
async def test_in_memory_ann_index_topk():
    idx = embed_mod.in_memory_ann_index(3)
    await idx.upsert(1, [1.0, 0.0, 0.0])
    await idx.upsert(2, [0.0, 1.0, 0.0])
    await idx.upsert(3, [0.7, 0.7, 0.0])
    hits = await idx.search([1.0, 0.0, 0.0], 2)
    assert hits[0][0] == 1
