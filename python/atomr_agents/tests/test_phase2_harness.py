"""Smoke tests for Phase 2.4 harness runtime bindings."""

from __future__ import annotations

import pytest

from atomr_agents import callable_ as callable_mod
from atomr_agents import harness as harness_mod


def _make_three_step_callable():
    """Return a callable that emits {"done": ...} on its 3rd call."""

    state = {"n": 0}

    def step(input, ctx):
        state["n"] += 1
        if state["n"] >= 3:
            return {"done": f"stopped@{state['n']}"}
        return state["n"]

    return callable_mod.Callable.from_callable(step)


@pytest.mark.asyncio
async def test_harness_runs_callable_loop_to_done():
    spec = harness_mod.HarnessSpec("h1", "0.1.0", initial_token_budget=100)
    loop = harness_mod.loop_strategy_from_callable(_make_three_step_callable())
    term = harness_mod.iteration_cap(10)
    h = harness_mod.Harness(spec, loop, term)
    out = await h.run()
    assert out == "stopped@3"


@pytest.mark.asyncio
async def test_harness_terminates_on_iteration_cap():
    # Callable that never emits done.
    forever = callable_mod.Callable.from_callable(lambda v, c: (v or 0) + 1)
    spec = harness_mod.HarnessSpec("h2", "0.1.0", initial_token_budget=100)
    loop = harness_mod.loop_strategy_from_callable(forever)
    term = harness_mod.iteration_cap(3)
    h = harness_mod.Harness(spec, loop, term)
    # iteration_cap terminates and returns working_memory at that point.
    out = await h.run()
    # working_memory after 3 iterations of "+ 1" starting from null is 3.
    assert out == 3
