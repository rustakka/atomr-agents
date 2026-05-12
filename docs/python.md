# Python bindings

`atomr-agents` ships a Python facade as `pip install atomr-agents`.
The native module is `atomr_agents._native` (PyO3); the user-facing
package lives at `python/atomr_agents/`.

## Install

### From PyPI (consumers)

```bash
pip install atomr-agents
```

### From a local checkout (developers)

```bash
pip install maturin
maturin develop --manifest-path crates/py-bindings/Cargo.toml
```

`maturin develop` builds the native extension in-place and installs
the Python wrapper from `python/atomr_agents/`.

## Host mode

Host mode is the *Python-drives-Rust* pattern: Python constructs
objects, kicks off runs, and reads results back. The native module
exposes the full framework surface — composition (`Callable`,
`Pipeline`, decorators), agent / workflow / harness runtimes,
retrievers, ingest pipelines, eval suites, conversation sessions,
plus the foundational `EventBus` and `Registry`:

```python
from atomr_agents import EventBus, Registry

# EventBus — subscribe a Python callable to receive every emitted event
bus = EventBus()
bus.subscribe(lambda ev: print(ev.kind, ev.timestamp_ms))
bus.emit_tool_invoked("calc", args_hash=0, elapsed_ms=5, ok=True)

# Registry — publish/get versioned artifacts
r = Registry()
r.publish("tool_set", "ts", "0.1.0", {"tools": ["calc", "search"]})
print(r.latest("tool_set", "ts"))
# {"id": "ts", "version": "0.1.0", "payload": {"tools": ["calc", "search"]}}

# Eval-gated publish blocks regression
try:
    r.publish_gated(
        "harness", "ch", "0.2.0", {"id": "ch"},
        current_pass_rate=0.50,
        baseline_pass_rate=0.95,
        tolerance=0.05,
    )
except RuntimeError as e:
    print("blocked:", e)
```

The agent / harness / workflow runtimes are callable from Python via
`AgentBuilder`, `Harness`, and `WorkflowRunner`:

```python
from atomr_agents.agent import AgentBuilder
from atomr_agents.harness import Harness, iteration_cap, loop_strategy_from_callable
from atomr_agents.workflow import Dag, Step, WorkflowRunner

builder = AgentBuilder("agent-1", "gpt-4o-mini")
builder.with_instructions(instr_strategy)
builder.with_tools(tool_strategy)
builder.with_memory(mem_strategy)
builder.with_skills(skill_strategy)
builder.with_inference(inference_client)
ref = builder.build()
result = await ref.run_turn("hello")

# Workflow runner — every step accepts any Callable, including agents.
dag = Dag("entry")
dag.add_step("entry", Step.invoke(ref.as_callable()))
runner = WorkflowRunner("wf-1", dag.build())
await runner.run({"user": "hello"})

# Harness loop driven by a Callable that emits {"done": value} to exit.
loop = loop_strategy_from_callable(some_callable)
term = iteration_cap(10)
h = Harness(spec, loop, term)
await h.run()
```

See [`python-api.md`](python-api.md) for the full submodule map.

## Guest mode

Guest mode is the *Python-defined-strategy-runs-inside-Rust-actors*
pattern. The Python wrapper provides marker decorators that register
factories with the native side:

```python
from atomr_agents.guest import tool, strategy, persona

@tool(toolset="finance")
def discounted_cash_flow(cashflows: list[float], rate: float) -> float:
    return sum(cf / (1 + rate) ** i for i, cf in enumerate(cashflows))

@strategy(kind="memory")
class MyMemoryStrategy:
    async def retrieve(self, ctx, budget):
        ...

@persona(name="brand-voice")
class BrandVoicePersona:
    async def resolve(self, ctx, budget):
        ...
```

The Rust side runs these factories on atomr's
`python-subinterpreter-pool` dispatcher (PEP 684 subinterpreters,
each on its own OS thread with its own GIL). This means:

- **The GIL is contained.** Python handlers run inside an isolated
  interpreter; the Rust scheduler hot path never crosses the FFI
  boundary while holding the GIL.
- **CPU-heavy Python releases the GIL through native extensions.**
  numpy / torch / pandas stay parallel.
- **Inference calls short-circuit to the Rust CUDA/inference actors
  directly.** A Python-defined agent loop still gets native-speed
  model calls without crossing the FFI boundary on the hot path.

The factory wiring uses the same pattern as
[atomr's pycore `PyActor`](https://github.com/rustakka/atomr/tree/main/crates/py-bindings/pycore)
— same `InterpreterKind::SubinterpreterPool` dispatcher, same
`errors::map` helper for translating panics.

## Async patterns

`pyo3-async-runtimes` bridges Rust futures to Python awaitables.
Inside the native module, async Rust calls return Python `coroutine`s
that integrate with `asyncio`:

```python
import asyncio
from atomr_agents import EventBus

async def main():
    bus = EventBus()
    bus.subscribe(lambda ev: ...)
    # Future host-mode entry points return awaitables.

asyncio.run(main())
```

For long-running calls (`Harness.run`), the eventual host-mode
surface uses `tokio::spawn` + `pyo3-async-runtimes::tokio::future_into_py`
so the returned coroutine awaits the in-flight Rust future without
blocking the Python event loop.

## Conventions

These follow atomr's pycore conventions:

1. **Module naming** — `atomr_agents` (snake_case Python package),
   `atomr_agents._native` (PyO3 cdylib), `atomr-agents` (PyPI dist
   name and crates.io umbrella).
2. **Async runtime** — `pyo3-async-runtimes::tokio::future_into_py`
   for coroutines; `py.allow_threads()` around blocking Rust I/O.
3. **Interpreter isolation** — `InterpreterKind::SubinterpreterPool`
   for parallel handler execution; pin GIL-contending handlers via
   stable hash.
4. **Error mapping** — Rust `Result<T, AgentError>` → Python
   `RuntimeError` via the helper that surfaces the underlying message
   without panicking the interpreter.
5. **Buffer management** — for GPU / compute-intensive paths, mirror
   `atomr-accel-py`'s pattern: blocking `System.open()` on the global
   tokio runtime, exposed as ergonomic Python objects.
6. **Feature flag** — the py-bindings crate is gated behind
   `extension-module` so the workspace builds without a Python venv.

## Where to go from here

- [Agent pipeline](agent-pipeline.md) — the Rust side of the surface
  the bindings expose.
- atomr's [`docs/python.md`](https://github.com/rustakka/atomr/blob/main/docs/python.md)
  — the GIL-strategy primer that atomr-agents follows.
