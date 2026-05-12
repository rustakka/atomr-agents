"""Smoke tests for Phase 2.3 workflow runtime bindings."""

from __future__ import annotations

import pytest

from atomr_agents import callable_ as callable_mod
from atomr_agents import workflow as wf


@pytest.mark.asyncio
async def test_single_step_workflow():
    add_one = callable_mod.Callable.from_callable(
        lambda v, c: v + 1, label="add1"
    )
    dag = wf.Dag("start")
    step = wf.Step.invoke(add_one)
    dag.add_step("start", step)
    handle = dag.build()
    runner = wf.WorkflowRunner("test-wf", handle)
    out = await runner.run(0)
    assert out == 1


@pytest.mark.asyncio
async def test_workflow_runner_as_callable():
    echo = callable_mod.Callable.identity()
    dag = wf.Dag("step")
    dag.add_step("step", wf.Step.invoke(echo))
    handle = dag.build()
    runner = wf.WorkflowRunner("echo-wf", handle)
    c = runner.as_callable()
    assert await c.call("hello") == "hello"


def test_in_memory_journal_constructs():
    j = wf.in_memory_journal()
    assert j is not None
