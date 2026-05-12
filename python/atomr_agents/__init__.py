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
    callable_ = _native.callable  # noqa: A001 - matches submodule name
    strategy = _native.strategy
    instruction = _native.instruction
    memory = _native.memory
    embed = _native.embed
    retriever = _native.retriever
    ingest = _native.ingest
    errors = _native.errors
    observability = _native.observability
    registry = _native.registry
    tool = _native.tool
    skill = _native.skill
    persona = _native.persona
    parser = _native.parser
    cache = _native.cache
    state = _native.state
    context = _native.context
    agent = _native.agent
    workflow = _native.workflow
    harness = _native.harness
    org = _native.org
    eval = _native.eval  # noqa: A001 - shadowing intentional
    guest = _native.guest
    stt = _native.stt
    tts = _native.tts
    voice = _native.voice
    voice_extras = _native.voice_extras

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
    AgentBuilder = agent.AgentBuilder
    AgentRef = agent.AgentRef
    InferenceClient = agent.InferenceClient
    AgentMiddleware = agent.AgentMiddleware
    logging_middleware = agent.logging_middleware
    tool_error_recovery_middleware = agent.tool_error_recovery_middleware
    redaction_middleware = agent.redaction_middleware
    rate_limit_middleware = agent.rate_limit_middleware

    StepKind = workflow.StepKind
    Dag = workflow.Dag
    Step = workflow.Step
    StepId = workflow.StepId
    WorkflowRunner = workflow.WorkflowRunner
    WorkflowState = workflow.WorkflowState
    Journal = workflow.Journal
    in_memory_journal = workflow.in_memory_journal

    HarnessSpec = harness.HarnessSpec
    IterationCapTermination = harness.IterationCapTermination
    Harness = harness.Harness
    HarnessState = harness.HarnessState
    StepEvent = harness.StepEvent
    LoopStrategy = harness.LoopStrategy
    TerminationStrategy = harness.TerminationStrategy
    iteration_cap = harness.iteration_cap

    PairwiseChoice = eval.PairwiseChoice
    Verdict = eval.Verdict

    # ----- callable -----------------------------------------------------
    Callable = callable_.Callable
    Pipeline = callable_.Pipeline
    with_retry = callable_.with_retry
    with_timeout = callable_.with_timeout
    with_fallbacks = callable_.with_fallbacks
    with_config = callable_.with_config
    fan_out = callable_.fan_out
    branch = callable_.branch
    lambda_ = callable_.lambda_
    passthrough = callable_.passthrough

    # ----- strategy -----------------------------------------------------
    Termination = strategy.Termination
    RoutingTarget = strategy.RoutingTarget
    SkillRef = strategy.SkillRef
    ToolRef = strategy.ToolRef
    Policy = strategy.Policy
    PolicyDecision = strategy.PolicyDecision

try:
    __version__ = _metadata.version("atomr-agents")
except _metadata.PackageNotFoundError:  # editable installs / running from source
    __version__ = "0.0.0+unknown"

__all__ = [
    # subpackages
    "core",
    "callable_",
    "strategy",
    "errors",
    "observability",
    "registry",
    "retriever",
    "ingest",
    "tool",
    "skill",
    "persona",
    "agent",
    "workflow",
    "harness",
    "org",
    "eval",
    "guest",
    "stt",
    "tts",
    "voice",
    "voice_extras",
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
    # callable
    "Callable",
    "Pipeline",
    "with_retry",
    "with_timeout",
    "with_fallbacks",
    "with_config",
    "fan_out",
    "branch",
    "lambda_",
    "passthrough",
]
