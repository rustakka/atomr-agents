"""Host-mode entry points.

Re-exports the Python-callable types that host code (drives Rust
strategies / agents from Python) needs. The actual classes live in
:mod:`atomr_agents._native.<submodule>`; this file is a thin facade
that mirrors atomr-infer's ``host.py`` shape.

Async APIs already wired in 0.3.0:

- :py:meth:`atomr_agents.Registry.publish_async` — coroutine
- :py:meth:`atomr_agents.RunTreeBuilder.flush_jsonl` — coroutine
- :py:meth:`atomr_agents.RunTreeBuilder.flush_langsmith` — coroutine
- :py:meth:`atomr_agents.RunTreeBuilder.flush_stdout` — coroutine
- :py:meth:`atomr_agents.EventBus.stream` — returns
  :class:`~atomr_agents.EventStream`, which supports ``async for``

Future async surfaces (``Agent.run_turn``, ``Harness.run``,
``WorkflowRunner.run``) ride on a ``Boxed*`` Rust adapter that lands
in a follow-up — until then, host-side code drives the agent loop in
Rust and observes via :class:`EventBus`.
"""

from . import (
    AgentBudgets,
    AgentSpec,
    ArtifactKind,
    ArtifactRecord,
    Event,
    EventBus,
    EventStream,
    EvalSummary,
    HarnessSpec,
    IterationCapTermination,
    PairwiseChoice,
    Provider,
    Registry,
    RenderedPersona,
    RunTreeBuilder,
    Skill,
    SkillSet,
    StepKind,
    ToolDescriptor,
    ToolSchema,
    ToolSet,
    TurnResult,
    Verdict,
)

__all__ = [
    "AgentBudgets",
    "AgentSpec",
    "ArtifactKind",
    "ArtifactRecord",
    "Event",
    "EventBus",
    "EventStream",
    "EvalSummary",
    "HarnessSpec",
    "IterationCapTermination",
    "PairwiseChoice",
    "Provider",
    "Registry",
    "RenderedPersona",
    "RunTreeBuilder",
    "Skill",
    "SkillSet",
    "StepKind",
    "ToolDescriptor",
    "ToolSchema",
    "ToolSet",
    "TurnResult",
    "Verdict",
]
