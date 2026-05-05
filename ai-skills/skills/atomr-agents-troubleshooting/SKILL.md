---
name: atomr-agents-troubleshooting
description: Use when debugging atomr-agents-flavored errors — `BudgetExceeded`, `PolicyDenied`, parser failures, retry exhaustion, channel mismatches, missing checkpoints, breakpoint loops, parallel-tool ordering bugs. Triggers on the error variants of `AgentError`, panicking workflow runs, missing run-id traces, or "why isn't my agent calling tools".
---

# Troubleshooting atomr-agents

A symptom-first guide. Most atomr-agents errors flow through
`AgentError`; this skill maps the common variants and the patterns
that produce them.

## `AgentError::BudgetExceeded(kind)`

| `kind` | What happened | Fix |
|---|---|---|
| `"tokens"` | A strategy or `ContextAssembler::assemble` tried to consume more than the remaining `TokenBudget` | Increase `AgentBudgets::tokens`, or trim instructions / memory. The assembler evicts low-priority fragments first; raise priorities of must-survive fragments. |
| `"time"` | `TimeBudget::consume` overflowed | Wrap with `with_timeout` for hard cap; rare in practice — `TimeBudget` is informational unless you enforce it explicitly |
| `"money"` | `MoneyBudget::consume_micro` overflowed | Reduce `max_tokens` on `SamplingParams`; pick a cheaper model |
| `"iterations"` | `IterationBudget::consume_one` ran out — usually the agent's tool-call loop | Bump `max_tool_iterations` on the agent, or set `IterationBudget::new(N)` higher in `AgentBudgets` |

## `AgentError::PolicyDenied(reason)`

Origins:

- **`NamespacedMemory` write rules.** "agents cannot write to
  org-level memory" / "write to team namespace t-1 denied". Use the
  agent's own namespace, or set `with_team_write(true)` if you
  legitimately want team-scratchpad writes.
- **`Registry::publish_gated`.** "regression: pass_rate 0.50 <
  baseline 0.95 - tol 0.05". Either fix the underlying eval
  regression, or bump the tolerance with intent.

## `AgentError::Tool(msg)`

| Message pattern | Fix |
|---|---|
| `"tool args parse: …"` | Model emitted malformed JSON in tool args. Wrap the model in `OutputFixingParser`-style retry or add `ToolErrorRecoveryMiddleware` |
| `"unknown tool: <name>"` | Tool name doesn't appear in the agent's `ToolStrategy`. Check `StaticToolStrategy::new(...)` includes it |
| `"missing 'key'"` (memory tools) | Model omitted a required arg. Tighten the descriptor schema |
| `"openai: tool_call missing index"` | Bad `tool_call_delta`. Ensure `Provider::OpenAi` is set on `LocalRunnerClient` |
| `"anthropic: delta missing index"` | Same, but Anthropic-shaped |

## `AgentError::Inference(msg)`

| Pattern | Fix |
|---|---|
| `"primary down"` / provider-specific 5xx | Wrap with `with_fallbacks(primary, vec![cheap_alt])` |
| `"rate-limit: gave up after 10s"` | `RateLimitMiddleware` capacity too small. Increase `capacity` / `refill_per_sec` |
| `"timed out after 5s"` | `with_timeout` fired. Either increase or tier with `with_retry` |
| `"retry exhausted with no error"` | `RetryPolicy::max_attempts = 0`. Set ≥ 1 |

## `AgentError::Workflow(msg)`

| Pattern | Fix |
|---|---|
| `"dag has a cycle"` | `Dag::topo_sort` detected cycle. Remove the offending edge |
| `"missing step <id>"` | Edge points at a step that isn't in `dag.steps` |
| `"resume: no checkpoint"` | `Interruptible::resume` called before `run`, or `(workflow_id, run_id)` mismatch |
| `"subgraph: expected object input"` | `Subgraph::call` requires a JSON object so the channel projection works |
| `"parallel: unknown <id>"` | `Step::Parallel { steps: [..] }` references a step not in the dag |

## State / channel mismatches

- **`"unknown channel '<name>'"`**: writing to a channel not declared
  in `StateSchema`. Add `.add(name, reducer)` or pick the right key.
- **Parallel writes producing wrong totals**: reducer isn't
  associative. Switch to `LastWriteWins` or rewrite the reducer.
- **Resume re-runs a step**: `(workflow_id, run_id)` differs across
  runs. Use a stable `RunId` for the same conversation thread.
- **Snapshot lost on resume**: `InMemoryCheckpointer` is per-process.
  Use `SqliteCheckpointer` / `PostgresCheckpointer` (feature-gated)
  for durability across restarts.

## Interrupt / breakpoint loops

- **Static breakpoint re-fires on resume**: not a bug — atomr-agents
  disables the *next* breakpoint hit on resume. If it still loops,
  remove the step from `interrupt_before`/`_after` after the first
  pause.
- **Dynamic interrupt loops forever**: step doesn't call
  `ctrl.take_resume_value()` on the resume path. Fix:

  ```rust
  if let Some(v) = ctrl.take_resume_value() {
      return Ok(/* normal output using v */);
  }
  ctrl.interrupt(...);
  Ok(vec![])
  ```

## Parallel tool ordering surprises

The agent fans tool calls into `tokio::JoinSet` but aggregates by
**original index** — so even if `search` finishes before `calc`,
the `Role::Tool` messages append in the order the model emitted
them. If your code depends on completion order, redesign: tools in
one turn should be independent.

## Missing tool calls

| Symptom | Likely cause |
|---|---|
| Model says "I'd call X" but no tool fires | `Provider` mismatch — the streaming parser doesn't recognize the delta format |
| Tool fires but args are empty | Schema doesn't match what model emits; tighten the JSON-Schema |
| Tool fires twice | Re-issuing the batch with stale assistant message; check `max_tool_iterations` |

## Parser failures (R9)

`SchemaParser<T>` returns `AgentError::Tool("schema parse: …")` on
malformed JSON. Wrap with `OutputFixingParser` or
`RetryWithErrorParser`:

```rust
let fixing: OutputFixingParser<SchemaParser<Plan>, Plan> =
    OutputFixingParser::new(parser, repair_model, /* max_attempts */ 3);
```

The repair model receives the malformed output + the parser's
format instructions and must emit corrected output.

## "Why is my agent silent?"

Three likely causes:

1. `MockRunner` script ran out of chunks. Append more `MockScript::from_text([...])`.
2. Real model returned `FinishReason::Stop` with empty `text` (rare; usually a model-side guardrail). Check `r.usage` and `r.finish_reason` on `TurnResult`.
3. Tool-call loop exited early because tool dispatch errored and
   `ToolErrorRecoveryMiddleware` isn't installed. Add it to the
   middleware stack.

## Diagnosing with `EventBus`

The fastest way to see *what's actually happening* in a turn is to
attach a `RunTreeBuilder` + `StdoutTracer`:

```rust
let bus = EventBus::new();
let builder = Arc::new(RunTreeBuilder::new());
builder.clone().attach(&bus);

let agent = Agent { /* ... */, bus: bus.clone(), /* ... */ };
let _ = agent.run_turn(...).await?;

StdoutTracer::new(builder).flush().await?;
```

Every strategy resolution, tool invocation, and turn boundary
prints with elapsed_ms. If something's missing from the tree, that
phase isn't running.

## Canonical references

- [`crates/core/src/error.rs`](https://github.com/rustakka/atomr-agents/blob/main/crates/core/src/error.rs) — `AgentError` definition
- [`docs/observability.md`](https://github.com/rustakka/atomr-agents/blob/main/docs/observability.md) — diagnosing via run tree
- [`docs/state-and-checkpointing.md`](https://github.com/rustakka/atomr-agents/blob/main/docs/state-and-checkpointing.md) — channel + checkpoint semantics
- atomr's [`docs/idiomatic-rust.md`](https://github.com/rustakka/atomr/blob/main/docs/idiomatic-rust.md) — the substrate's error/panic conventions

## Common mistakes summary

- **Mixed `Provider`s** in a multi-model agent — pick per-runner.
- **Empty `tools`** with `max_tool_iterations > 1` wastes inference budget.
- **Reducer isn't associative** — non-deterministic parallel results.
- **Different `RunId` on resume** — fresh run, no checkpoint hit.
- **Forgetting `take_resume_value()`** in an interrupt step — infinite loop.
- **Subscribing to `EventBus` after emit** — you miss events.
