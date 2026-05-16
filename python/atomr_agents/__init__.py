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
    stt_harness = _native.stt_harness
    coding_cli = _native.coding_cli
    channel = _native.channel
    tts = _native.tts
    voice = _native.voice
    voice_extras = _native.voice_extras
    # The avatar submodule is x86_64-only and feature-gated; on arm64
    # wheels (or wheels built without `--features avatar`) the attribute
    # won't exist, so resolve it defensively.
    avatar = getattr(_native, "avatar", None)

    # ----- top-level convenience re-exports -----------------------------
    # Each name is resolved defensively: the native submodule surface
    # evolves independently of this facade, so a name that is not (yet)
    # bound on the Rust side degrades to `None` rather than crashing the
    # whole package import.
    def _opt(_module, _name):
        return getattr(_module, _name, None)

    AgentId = _opt(core, "AgentId")
    TeamId = _opt(core, "TeamId")
    DepartmentId = _opt(core, "DepartmentId")
    OrgId = _opt(core, "OrgId")
    WorkflowId = _opt(core, "WorkflowId")
    HarnessId = _opt(core, "HarnessId")
    ToolId = _opt(core, "ToolId")
    ToolSetId = _opt(core, "ToolSetId")
    SkillId = _opt(core, "SkillId")
    PersonaId = _opt(core, "PersonaId")
    RunId = _opt(core, "RunId")
    TokenBudget = _opt(core, "TokenBudget")
    TimeBudget = _opt(core, "TimeBudget")
    MoneyBudget = _opt(core, "MoneyBudget")
    IterationBudget = _opt(core, "IterationBudget")
    MemoryNamespace = _opt(core, "MemoryNamespace")
    MemoryKind = _opt(core, "MemoryKind")
    MemoryItem = _opt(core, "MemoryItem")
    MemoryChunk = _opt(core, "MemoryChunk")
    TokenUsage = _opt(core, "TokenUsage")
    FinishReason = _opt(core, "FinishReason")

    Event = _opt(observability, "Event")
    EventBus = _opt(observability, "EventBus")
    EventStream = _opt(observability, "EventStream")
    RunTreeBuilder = _opt(observability, "RunTreeBuilder")

    Registry = _opt(registry, "Registry")
    ArtifactKind = _opt(registry, "ArtifactKind")
    ArtifactRecord = _opt(registry, "ArtifactRecord")
    EvalSummary = _opt(registry, "EvalSummary")

    ToolDescriptor = _opt(tool, "ToolDescriptor")
    ToolSchema = _opt(tool, "ToolSchema")
    ToolSet = _opt(tool, "ToolSet")
    Provider = _opt(tool, "Provider")
    ParsedToolCall = _opt(tool, "ParsedToolCall")
    ToolCallParser = _opt(tool, "ToolCallParser")

    Skill = _opt(skill, "Skill")
    SkillSet = _opt(skill, "SkillSet")

    RenderedPersona = _opt(persona, "RenderedPersona")

    AgentSpec = _opt(agent, "AgentSpec")
    AgentBudgets = _opt(agent, "AgentBudgets")
    TurnResult = _opt(agent, "TurnResult")
    AgentBuilder = _opt(agent, "AgentBuilder")
    AgentRef = _opt(agent, "AgentRef")
    InferenceClient = _opt(agent, "InferenceClient")
    AgentMiddleware = _opt(agent, "AgentMiddleware")
    logging_middleware = _opt(agent, "logging_middleware")
    tool_error_recovery_middleware = _opt(agent, "tool_error_recovery_middleware")
    redaction_middleware = _opt(agent, "redaction_middleware")
    rate_limit_middleware = _opt(agent, "rate_limit_middleware")

    StepKind = _opt(workflow, "StepKind")
    Dag = _opt(workflow, "Dag")
    Step = _opt(workflow, "Step")
    StepId = _opt(workflow, "StepId")
    WorkflowRunner = _opt(workflow, "WorkflowRunner")
    WorkflowState = _opt(workflow, "WorkflowState")
    Journal = _opt(workflow, "Journal")
    in_memory_journal = _opt(workflow, "in_memory_journal")

    HarnessSpec = _opt(harness, "HarnessSpec")
    IterationCapTermination = _opt(harness, "IterationCapTermination")
    Harness = _opt(harness, "Harness")
    HarnessState = _opt(harness, "HarnessState")
    StepEvent = _opt(harness, "StepEvent")
    LoopStrategy = _opt(harness, "LoopStrategy")
    TerminationStrategy = _opt(harness, "TerminationStrategy")
    iteration_cap = _opt(harness, "iteration_cap")

    PairwiseChoice = _opt(eval, "PairwiseChoice")
    Verdict = _opt(eval, "Verdict")

    # ----- callable -----------------------------------------------------
    Callable = _opt(callable_, "Callable")
    Pipeline = _opt(callable_, "Pipeline")
    with_retry = _opt(callable_, "with_retry")
    with_timeout = _opt(callable_, "with_timeout")
    with_fallbacks = _opt(callable_, "with_fallbacks")
    with_config = _opt(callable_, "with_config")
    fan_out = _opt(callable_, "fan_out")
    branch = _opt(callable_, "branch")
    lambda_ = _opt(callable_, "lambda_")
    passthrough = _opt(callable_, "passthrough")

    # ----- strategy -----------------------------------------------------
    Termination = _opt(strategy, "Termination")
    RoutingTarget = _opt(strategy, "RoutingTarget")
    SkillRef = _opt(strategy, "SkillRef")
    ToolRef = _opt(strategy, "ToolRef")
    Policy = _opt(strategy, "Policy")
    PolicyDecision = _opt(strategy, "PolicyDecision")

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
    "stt_harness",
    "tts",
    "voice",
    "voice_extras",
    "avatar",
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
