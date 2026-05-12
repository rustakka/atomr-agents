"""Smoke tests for Phase 1.1 callable bindings."""

from __future__ import annotations

import asyncio

import pytest

from atomr_agents import callable_ as callable_mod


def test_identity_callable_round_trips():
    c = callable_mod.Callable.identity()
    result = c.call_sync({"hello": "world"})
    assert result == {"hello": "world"}


def test_from_callable_with_sync_python_fn():
    def double(input, ctx):
        return {"out": input["x"] * 2}

    c = callable_mod.Callable.from_callable(double)
    result = c.call_sync({"x": 21})
    assert result == {"out": 42}


@pytest.mark.asyncio
async def test_from_callable_with_async_python_fn():
    async def async_fn(input, ctx):
        await asyncio.sleep(0)
        return input

    c = callable_mod.Callable.from_callable(async_fn)
    out = await c.call({"a": 1})
    assert out == {"a": 1}


def test_pipeline_then():
    add_a = callable_mod.Callable.from_callable(
        lambda v, c: v + "A", label="addA"
    )
    add_b = callable_mod.Callable.from_callable(
        lambda v, c: v + "B", label="addB"
    )
    p = callable_mod.Pipeline.from_(add_a)
    p.then(add_b)
    built = p.build()
    assert built.call_sync("") == "AB"


def test_passthrough():
    c = callable_mod.passthrough()
    assert c.call_sync(42) == 42


def test_with_retry_succeeds_after_failures():
    attempts = []

    def flaky(input, ctx):
        attempts.append(input)
        if len(attempts) < 3:
            raise RuntimeError("boom")
        return "ok"

    flaky_callable = callable_mod.Callable.from_callable(flaky)
    retried = callable_mod.with_retry(
        flaky_callable,
        max_attempts=5,
        initial_backoff_ms=1,
        backoff_multiplier=1.0,
        max_backoff_ms=1,
    )
    assert retried.call_sync(None) == "ok"
    assert len(attempts) == 3
