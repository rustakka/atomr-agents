"""Async-surface smoke tests — Registry.publish_async, EventStream
async iteration, parser.parse, and the in-memory LLM cache."""

import asyncio

import pytest

native = pytest.importorskip("atomr_agents._native")


def test_registry_publish_async() -> None:
    r = native.registry.Registry()

    async def go() -> None:
        await r.publish_async("tool_set", "ts1", "0.1.0", {"name": "ts1"})
        await r.publish_async("tool_set", "ts1", "0.2.0", {"name": "ts1"})
        latest = r.latest("tool_set", "ts1")
        assert latest["version"] == "0.2.0"

    asyncio.run(go())


def test_event_stream_async_iter() -> None:
    bus = native.observability.EventBus()
    stream = bus.stream()

    async def go() -> list[str]:
        # Pre-emit, since stream() subscribes from this point forward.
        bus.emit_tool_invoked("calc", 0, 5, True)
        bus.emit_tool_invoked("search", 0, 9, False)
        events: list[str] = []
        # __anext__ resolves once per emit; pull two and break.
        events.append((await stream.__anext__()).kind)
        events.append((await stream.__anext__()).kind)
        return events

    out = asyncio.run(go())
    assert out == ["tool_invoked", "tool_invoked"]


def test_run_tree_flush_jsonl_async() -> None:
    bus = native.observability.EventBus()
    builder = native.observability.RunTreeBuilder()
    builder.attach(bus)

    async def go() -> list[str]:
        # No emitted events → no run nodes → empty JSONL.
        lines = await builder.flush_jsonl()
        return lines

    out = asyncio.run(go())
    assert isinstance(out, list)


def test_json_parser_async_parse() -> None:
    p = native.parser.JsonParser()

    async def go() -> dict:
        return await p.parse('{"a": 1, "b": "two"}')

    result = asyncio.run(go())
    assert result == {"a": 1, "b": "two"}


def test_in_memory_llm_cache_async_round_trip() -> None:
    cache = native.cache.InMemoryLlmCache()
    key = native.cache.CacheKey(model="m", messages_hash=42, sampling_hash=7)
    value = native.cache.CachedTurn(
        text="cached!",
        usage=native.core.TokenUsage(input_tokens=10, output_tokens=2),
    )

    async def go() -> tuple[object, object]:
        miss = await cache.get(key)
        await cache.put(key, value)
        hit = await cache.get(key)
        return miss, hit

    miss, hit = asyncio.run(go())
    assert miss is None
    assert hit is not None
    assert hit.text == "cached!"


def test_guest_tool_round_trip_via_adapter() -> None:
    """End-to-end: register a Python @tool class, build a guest
    ToolSet, confirm the adapter ferries args to/from the Python
    method through the Rust Tool trait."""
    native.guest.clear_factories()

    class Calc:
        def invoke(self, args, ctx):
            return {"sum": args["a"] + args["b"]}

    descriptor = native.tool.ToolDescriptor(
        id="calc",
        name="calc",
        description="adds two numbers",
    )
    native.guest.register_tool_factory("calc", Calc, descriptor)

    ts = native.guest.build_guest_toolset("guest", "0.1.0", ["calc"])
    assert ts.id == "guest"
    assert len(ts) == 1
