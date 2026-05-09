"""atomr-agents — composable agentic framework on top of atomr.

The native PyO3 extension lives in :mod:`atomr_agents._native` and is
split into per-domain submodules — ``core``, ``errors``,
``observability``, ``registry``, ``tool``, ``skill``, ``persona``,
``agent``, ``workflow``, ``harness``, ``eval``, and ``guest`` —
mirroring the upstream ``atomr-infer`` / ``atomr/pycore`` Python
binding layout.

Build the native extension::

    pip install maturin
    maturin develop --features python -m crates/py-bindings/Cargo.toml

Common imports (host mode, top-level)::

    from atomr_agents import (
        AgentSpec, AgentBudgets,
        Registry, ArtifactKind, ArtifactRecord, EvalSummary,
        EventBus, Event,
        ToolDescriptor, ToolSet, Provider,
        Skill, SkillSet,
        HarnessSpec, IterationCapTermination,
    )
    from atomr_agents.errors import AgentError, RegistryError, BudgetExhausted

Async EventStream::

    bus = EventBus()
    stream = bus.stream()
    bus.emit_tool_invoked("calc", 0, 5, True)

    async def watch():
        async for ev in stream:
            print(ev.kind)

Async Registry::

    r = Registry()
    await r.publish_async("tool_set", "calc", "0.1.0", {"name": "calc"})
"""

from importlib import metadata as _metadata

try:
    from . import _native
except ImportError as _e:  # pragma: no cover - native extension not built yet
    _native = None
    _import_err = _e
else:
    _import_err = None

if _native is not None:
    # ----- subpackages re-exported as attributes ------------------------
    core = _native.core
    errors = _native.errors
    observability = _native.observability
    registry = _native.registry
    tool = _native.tool
    skill = _native.skill
    persona = _native.persona
    agent = _native.agent
    workflow = _native.workflow
    harness = _native.harness
    eval = _native.eval  # noqa: A001 - shadowing intentional
    guest = _native.guest

    # ----- top-level convenience re-exports -----------------------------
    AgentId = core.AgentId
    TeamId = core.TeamId
    DepartmentId = core.DepartmentId
    OrgId = core.OrgId
    WorkflowId = core.WorkflowId
    HarnessId = core.HarnessId
    ToolId = core.ToolId
    ToolSetId = core.ToolSetId
    SkillId = core.SkillId
    PersonaId = core.PersonaId
    RunId = core.RunId
    TokenBudget = core.TokenBudget
    TimeBudget = core.TimeBudget
    MoneyBudget = core.MoneyBudget
    IterationBudget = core.IterationBudget
    MemoryNamespace = core.MemoryNamespace
    MemoryKind = core.MemoryKind
    MemoryItem = core.MemoryItem
    MemoryChunk = core.MemoryChunk
    TokenUsage = core.TokenUsage
    FinishReason = core.FinishReason

    Event = observability.Event
    EventBus = observability.EventBus
    EventStream = observability.EventStream
    RunTreeBuilder = observability.RunTreeBuilder

    Registry = registry.Registry
    ArtifactKind = registry.ArtifactKind
    ArtifactRecord = registry.ArtifactRecord
    EvalSummary = registry.EvalSummary

    ToolDescriptor = tool.ToolDescriptor
    ToolSchema = tool.ToolSchema
    ToolSet = tool.ToolSet
    Provider = tool.Provider
    ParsedToolCall = tool.ParsedToolCall
    ToolCallParser = tool.ToolCallParser

    Skill = skill.Skill
    SkillSet = skill.SkillSet

    RenderedPersona = persona.RenderedPersona

    AgentSpec = agent.AgentSpec
    AgentBudgets = agent.AgentBudgets
    TurnResult = agent.TurnResult

    StepKind = workflow.StepKind

    HarnessSpec = harness.HarnessSpec
    IterationCapTermination = harness.IterationCapTermination

    PairwiseChoice = eval.PairwiseChoice
    Verdict = eval.Verdict

try:
    __version__ = _metadata.version("atomr-agents")
except _metadata.PackageNotFoundError:  # editable installs / running from source
    __version__ = "0.0.0+unknown"

__all__ = [
    # subpackages
    "core",
    "errors",
    "observability",
    "registry",
    "tool",
    "skill",
    "persona",
    "agent",
    "workflow",
    "harness",
    "eval",
    "guest",
    # core
    "AgentId",
    "TeamId",
    "DepartmentId",
    "OrgId",
    "WorkflowId",
    "HarnessId",
    "ToolId",
    "ToolSetId",
    "SkillId",
    "PersonaId",
    "RunId",
    "TokenBudget",
    "TimeBudget",
    "MoneyBudget",
    "IterationBudget",
    "MemoryNamespace",
    "MemoryKind",
    "MemoryItem",
    "MemoryChunk",
    "TokenUsage",
    "FinishReason",
    # observability
    "Event",
    "EventBus",
    "EventStream",
    "RunTreeBuilder",
    # registry
    "Registry",
    "ArtifactKind",
    "ArtifactRecord",
    "EvalSummary",
    # tool
    "ToolDescriptor",
    "ToolSchema",
    "ToolSet",
    "Provider",
    "ParsedToolCall",
    "ToolCallParser",
    # skill
    "Skill",
    "SkillSet",
    # persona
    "RenderedPersona",
    # agent
    "AgentSpec",
    "AgentBudgets",
    "TurnResult",
    # workflow
    "StepKind",
    # harness
    "HarnessSpec",
    "IterationCapTermination",
    # eval
    "PairwiseChoice",
    "Verdict",
]
