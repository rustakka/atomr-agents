"""Smoke tests for the native module — only runs after `maturin develop`."""

import pytest

native = pytest.importorskip("atomr_agents._native")


def test_module_layout() -> None:
    """The hierarchical _native.{...} submodules registered by the
    parity-wave restructure are all importable."""
    assert hasattr(native, "core")
    assert hasattr(native, "errors")
    assert hasattr(native, "observability")
    assert hasattr(native, "registry")
    assert hasattr(native, "tool")
    assert hasattr(native, "skill")
    assert hasattr(native, "persona")
    assert hasattr(native, "agent")
    assert hasattr(native, "workflow")
    assert hasattr(native, "harness")
    assert hasattr(native, "eval")
    assert hasattr(native, "guest")


def test_top_level_reexports() -> None:
    import atomr_agents as a

    # core
    assert a.AgentId is native.core.AgentId
    assert a.TokenBudget is native.core.TokenBudget
    # observability
    assert a.EventBus is native.observability.EventBus
    assert a.Event is native.observability.Event
    # registry
    assert a.Registry is native.registry.Registry
    assert a.ArtifactKind is native.registry.ArtifactKind
    # tool
    assert a.ToolDescriptor is native.tool.ToolDescriptor
    # agent
    assert a.AgentSpec is native.agent.AgentSpec


def test_event_bus_dispatch() -> None:
    bus = native.observability.EventBus()
    received: list = []
    bus.subscribe(lambda ev: received.append(ev.kind))
    bus.emit_tool_invoked("calc", 0, 5, True)
    assert received == ["tool_invoked"]


def test_registry_pin_and_latest() -> None:
    r = native.registry.Registry()
    r.publish("tool_set", "ts", "0.1.0", {"name": "ts"})
    r.publish("tool_set", "ts", "0.2.0", {"name": "ts"})
    latest = r.latest("tool_set", "ts")
    assert latest is not None
    assert latest["version"] == "0.2.0"


def test_publish_gated_blocks_regression() -> None:
    r = native.registry.Registry()
    with pytest.raises(native.errors.RegistryError):
        r.publish_gated(
            "harness",
            "h",
            "0.1.0",
            {"id": "h"},
            current_pass_rate=0.50,
            baseline_pass_rate=0.95,
            tolerance=0.05,
        )


def test_artifact_kind_string_tag() -> None:
    k = native.registry.ArtifactKind("tool_set")
    assert k.name == "tool_set"
    # staticmethod constructors
    assert native.registry.ArtifactKind.tool_set().name == "tool_set"
    assert native.registry.ArtifactKind.harness().name == "harness"
    with pytest.raises(ValueError):
        native.registry.ArtifactKind("bogus")


def test_token_budget_consume_and_split() -> None:
    b = native.core.TokenBudget(1000)
    b.consume(100)
    assert b.remaining == 900
    parts = b.split(3)
    assert len(parts) == 3
    assert parts[0].remaining == 300


def test_token_budget_exhausted_raises_typed_error() -> None:
    b = native.core.TokenBudget(10)
    with pytest.raises(native.errors.BudgetExhausted):
        b.consume(11)


def test_agent_spec_defaults_round_trip() -> None:
    spec = native.agent.AgentSpec(id="a-1", model="gpt-4o", token_budget=2000)
    assert spec.model == "gpt-4o"
    assert spec.token_budget == 2000
    bud = spec.default_budgets()
    assert bud.tokens.remaining == 2000


def test_tool_descriptor_round_trip() -> None:
    d = native.tool.ToolDescriptor(
        id="get_weather",
        name="get_weather",
        description="Fetches the weather.",
    )
    assert d.id == "get_weather"
    assert d.name == "get_weather"
    schema = d.schema()
    schema_dict = schema.to_dict()
    assert schema_dict["type"] == "object"


def test_skill_set_round_trip() -> None:
    s = native.skill.Skill(
        id="rag",
        name="RAG",
        instruction_fragment="use the index",
        keywords=["search"],
        priority=8,
    )
    set_ = native.skill.SkillSet(id="my-set", version="0.1.0", skills=[s])
    assert len(set_) == 1
    assert s.priority == 8


def test_harness_spec_round_trip() -> None:
    h = native.harness.HarnessSpec(id="h-1", version="0.1.0", initial_token_budget=500)
    assert h.id == "h-1"
    assert h.initial_token_budget == 500


def test_guest_factory_round_trip() -> None:
    native.guest.clear_factories()

    class Calc:
        def invoke(self, args, ctx):
            return {"sum": args["a"] + args["b"]}

    handle = native.guest.register_tool_factory("calc", Calc)
    assert handle.kind == "tool"
    assert handle.key == "calc"
    keys = native.guest.list_factories("tool")
    assert "calc" in keys


def test_step_kind_string_tag() -> None:
    s = native.workflow.StepKind.invoke()
    assert s.name == "invoke"
    with pytest.raises(ValueError):
        native.workflow.StepKind("bogus")
