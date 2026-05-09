# Python API reference

This page is the structural map of the `atomr_agents` Python
package as it ships in 0.3 — the "Python parity wave". The layout
mirrors the upstream
[`atomr-infer/inference-py-bindings`](https://github.com/rustakka/atomr-infer/tree/main/crates/inference-py-bindings/src)
and [`atomr/pycore`](https://github.com/rustakka/atomr/tree/main/crates/py-bindings/pycore)
binding crates so consumers who already use the sibling Python
surfaces find the same idioms here: a hierarchical `_native.{...}`
PyO3 module, one facade `.py` per submodule, async coroutines via
`pyo3-async-runtimes`, an exception hierarchy rooted at
`AgentError`, and a guest-mode decorator family that registers
Python implementations as Rust trait objects.

For the prose-level introduction to host vs guest mode and the GIL
strategy, read [`python.md`](python.md) first. This document is the
type-by-type module map.

## Module map

The native extension is `atomr_agents._native`. Every submodule has
a sibling facade `.py` under `python/atomr_agents/` that re-exports
its public surface, so user code never has to import from
`_native.*` directly.

| `atomr_agents.<submodule>` | Purpose | Key types |
|---|---|---|
| `errors` | Exception hierarchy translating Rust `AgentError` | `AgentError`, `RegistryError`, `BudgetExhausted`, `ToolError`, `StrategyError`, `WorkflowError`, `HarnessError`, `EvalError`, `MemoryError`, `ParserError`, `CacheError` |
| `core` | IDs, budgets, memory primitives, token usage | `AgentId`, `TeamId`, `DepartmentId`, `OrgId`, `WorkflowId`, `HarnessId`, `ToolId`, `ToolSetId`, `SkillId`, `PersonaId`, `RunId`; `TokenBudget`, `TimeBudget`, `MoneyBudget`, `IterationBudget`; `MemoryNamespace`, `MemoryKind`, `MemoryItem`, `MemoryChunk`; `TokenUsage`; `FinishReason` |
| `observability` | Event taxonomy, event bus, run-tree builder | `Event`, `EventBus`, `EventStream` (async iterator), `RunTreeBuilder` |
| `registry` | Versioned artifact registry | `Registry`, `ArtifactKind`, `ArtifactRecord`, `EvalSummary` |
| `tool` | Tool descriptors and provider-aware parser | `ToolSchema`, `ToolDescriptor`, `Provider`, `ParsedToolCall`, `ToolCallParser`, `ToolSet` |
| `skill` | Skill primitives | `Skill`, `SkillSet` |
| `persona` | Rendered persona output | `RenderedPersona` |
| `agent` | Agent specs and turn results | `AgentSpec`, `AgentBudgets`, `TurnResult` |
| `workflow` | Workflow step taxonomy | `StepKind` |
| `harness` | Harness specs and termination | `HarnessSpec`, `IterationCapTermination` |
| `eval` | Eval verdicts | `PairwiseChoice`, `Verdict` |
| `guest` | Python-implementable Rust trait factories | `GuestHandle`, `register_*_factory(...)`; user-facing `@tool` / `@strategy` / `@persona` / `@skill` / `@parser` / `@scorer` / `@memory_store` / `@embedder` decorators |

The top-level `atomr_agents.__init__` re-exports the user-facing
classes from each submodule, so `from atomr_agents import EventBus,
Registry, AgentSpec, TokenBudget` resolves without users having to
remember submodule paths. `__version__` is sourced from
`importlib.metadata`.

## Host mode

Host mode is the *Python-drives-Rust* pattern: Python constructs
PyO3-exposed Rust types, drives them, and reads results back. The
typical host-mode loop today is:

1. Build a `Registry()` and publish artifact descriptors.
2. Build an `EventBus()` and subscribe / async-iterate it for
   observability.
3. Construct `RunTreeBuilder` and flush to a tracer when a run
   completes.
4. (Roadmap) Construct an `AgentSpec` / `HarnessSpec` and call
   `.run(...)` / `.run_turn(...)` from Python.

Today the host-mode entry points cover the registry, observability,
and the descriptor/spec types. The agent / harness / workflow
runners themselves are not yet directly callable from Python —
see the roadmap below.

## Guest mode

Guest mode is the *Python-defined-strategy-runs-inside-Rust-actors*
pattern. The Python side defines a class (or function) that
implements a Rust trait — `ToolStrategy`, `MemoryStrategy`,
`SkillStrategy`, `PersonaStrategy`, `Parser<T>`, `Scorer`,
`MemoryStore`, `Embedder` — and decorates it with the matching
`atomr_agents.guest` decorator. The decorator calls
`_native.guest.register_*_factory(...)` to plant a factory that
the Rust scheduler invokes on each new actor instance.

```python
from atomr_agents.guest import tool, strategy, persona


@tool(toolset="finance")
class DiscountedCashFlow:
    name = "dcf"

    async def invoke(self, args: dict, ctx) -> dict:
        rate = args["rate"]
        cashflows = args["cashflows"]
        return {
            "npv": sum(cf / (1 + rate) ** i for i, cf in enumerate(cashflows)),
        }


@strategy(kind="memory")
class MyMemoryStrategy:
    async def retrieve(self, ctx, budget):
        ...


@persona(name="brand-voice")
class BrandVoicePersona:
    async def resolve(self, ctx, budget):
        ...
```

Guest factories run on the upstream
`python-subinterpreter-pool` dispatcher (PEP 684 subinterpreters,
each with its own GIL); see [`python.md`](python.md) for the GIL
strategy and isolation guarantees.

## Async surfaces today

Every async method below is exposed via
`pyo3-async-runtimes::tokio::future_into_py`, so calling it returns
a Python coroutine awaitable from any `asyncio` event loop without
blocking the Python thread.

- `Registry.publish_async(kind, id, version, payload) -> ArtifactRecord`
- `RunTreeBuilder.flush_stdout() -> None`
- `RunTreeBuilder.flush_jsonl(path) -> None`
- `RunTreeBuilder.flush_langsmith(endpoint, api_key) -> None`
- `EventBus.stream() -> EventStream` (the stream itself is an
  async iterator: `__aiter__` / `__anext__`)

Sync versions (`Registry.publish` / `Registry.get` /
`Registry.latest` / `Registry.list`, `EventBus.subscribe(callback)`,
`EventBus.emit_*`) remain available for non-async call sites.

## Roadmap

The following surfaces are designed but not yet wired to Python.
They all wait on the same upstream blocker: the Rust types are
generic over four-plus strategy traits, and PyO3 cannot construct
generics from a stable `#[pyclass]` shape, so each needs a
type-erased `Boxed*` adapter on the Rust side first.

- **`Agent.run_turn`** — needs `BoxedAgent` in `crates/agent`.
  Until then, host code constructs an `Agent<I, T, Ms, Sk>` in Rust
  and observes progress from Python via the `EventBus` async stream.
- **`Harness.run`** — needs `BoxedHarness` in `crates/harness`.
  The `HarnessSpec` descriptor is exposed today; spawning is not.
- **`WorkflowRunner.run`** — needs `BoxedWorkflow` in
  `crates/workflow`. `StepKind` is exposed today.
- **`atomr-pycore` subinterpreter-pool dispatcher for guests.**
  The factory plumbing lands today as a single-interpreter dispatch.
  Wiring it into `InterpreterKind::SubinterpreterPool` (per atomr
  `pycore`) so handlers run in isolated GILs lands in a follow-up.

Until the `Boxed*` adapters land, the recommended host-mode pattern
is:

```python
import asyncio
from atomr_agents.observability import EventBus


async def observe(bus: EventBus) -> None:
    async for ev in bus.stream():
        print(ev.kind, ev.timestamp_ms)


# Spawn the agent loop in Rust (CLI / service binary), then observe
# from Python via the bus.
```

## Migration notes for users on 0.2.x

The 0.2.x release exposed `_native.Event`, `_native.EventBus`, and
`_native.Registry` directly under the native module root. The 0.3
restructure moves these under hierarchical submodules:

| 0.2.x import | 0.3 location |
|---|---|
| `atomr_agents._native.Event` | `atomr_agents._native.observability.Event` |
| `atomr_agents._native.EventBus` | `atomr_agents._native.observability.EventBus` |
| `atomr_agents._native.Registry` | `atomr_agents._native.registry.Registry` |

The top-level package re-exports keep the 0.2.x convenience imports
working unchanged:

```python
# Both of these still work in 0.3:
from atomr_agents import EventBus, Registry

# And the new submodule path is preferred for new code:
from atomr_agents.observability import EventBus
from atomr_agents.registry import Registry
```

The `atomr_agents.guest` decorators (`@tool`, `@strategy`,
`@persona`) preserve their 0.2.x signatures, but they are no longer
no-op markers — they now register the decorated callable / class as
a real factory through `_native.guest.register_*_factory`. Code
that worked under 0.2.x continues to work; code that previously
relied on the marker being a no-op (e.g. importing the package in
an environment without the native extension) needs to guard the
import.

## Where to go from here

- [`python.md`](python.md) — host vs guest prose, GIL strategy,
  subinterpreter-pool dispatcher.
- [`observability.md`](observability.md) — Rust side of `EventBus`
  / `RunTreeBuilder` / tracers.
- [`agent-pipeline.md`](agent-pipeline.md) — what `Agent.run_turn`
  will drive once the Boxed adapter lands.
