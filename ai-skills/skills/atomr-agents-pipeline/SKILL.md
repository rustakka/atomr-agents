---
name: atomr-agents-pipeline
description: Use when composing `Callable`s with the `Pipeline` builder or wrapping any handle in retry / fallback / config / timeout / branch decorators. Triggers on writing `Pipeline::from(...).then(...)`, calling `with_retry` / `with_fallbacks` / `with_config` / `with_timeout` / `Branch::new`, fan-out into a JSON object, or migrating a LangChain LCEL chain.
---

# Composing callables in atomr-agents

`Callable` is the universal trait every executable unit implements
(prompts, models, parsers, retrievers, tools, sub-agents, workflows,
harnesses). `Pipeline` chains callables; decorators wrap them.

## Mental model

A pipeline is a vector of stages. `then` appends a sequential stage
(output → next input). `fan_out_with` runs N branches concurrently
and returns a JSON object keyed by branch name. `assign(key, c)`
runs `c` on the *current* input and adds the result under `key`,
preserving original input fields.

Decorators (`with_retry`, `with_fallbacks`, `with_config`,
`with_timeout`, `Branch`) are themselves `Callable`s wrapping another
`Callable`. They compose freely:

```rust
let resilient: CallableHandle = with_fallbacks(
    with_retry(primary, RetryPolicy::default()),
    vec![cheap_alt, expensive_alt],
);
```

## The basic pipe

LangChain LCEL:

```python
chain = prompt | model | parser
```

atomr-agents:

```rust
use atomr_agents::callable::Pipeline;

let chain = Pipeline::from(prompt)
    .then(model)
    .then(parser)
    .build();

let output = chain.call(input, ctx).await?;
```

`build()` returns a `CallableHandle` (an `Arc<dyn Callable>`).
Multiple consumers can hold and call it concurrently.

## Fan-out

```rust
use atomr_agents::callable::{Pipeline, fan_out};

let parallel: CallableHandle = fan_out(vec![
    ("summary".into(), summarize_chain),
    ("entities".into(), extract_entities),
    ("sentiment".into(), score_sentiment),
]);

let v = parallel.call(input, ctx).await?;
// v == {"summary": ..., "entities": [...], "sentiment": 0.7}
```

Or inline within a pipeline:

```rust
let chain = Pipeline::from(load_doc)
    .fan_out_with(vec![
        ("summary".into(),  summarize_chain),
        ("entities".into(), extract_entities),
    ])
    .then(merge_results)
    .build();
```

## Adding a derived key (`assign`)

```rust
let derive_count: CallableHandle = Arc::new(FnCallable::labeled(
    "count",
    |v: Value, _ctx| async move {
        Ok(Value::from(v.as_object().map(|m| m.len()).unwrap_or(0)))
    },
));

let chain = Pipeline::from(echo)
    .assign("size", derive_count)
    .build();

// Input  : {"a": 1, "b": 2}
// Output : {"a": 1, "b": 2, "size": 2}
```

## Retries

```rust
use atomr_agents::callable::{with_retry, RetryPolicy};
use std::time::Duration;

let retried = with_retry(
    flaky_handle,
    RetryPolicy {
        max_attempts: 3,
        initial_backoff: Duration::from_millis(100),
        backoff_multiplier: 2.0,
        max_backoff: Duration::from_secs(5),
    },
);
```

Defaults are sensible: 3 attempts, 50 ms initial backoff doubling
to a 5 s ceiling.

## Fallbacks

```rust
use atomr_agents::callable::with_fallbacks;

let model = with_fallbacks(
    primary_model,                    // try first
    vec![cheap_model, fast_model],     // alternates in order
);
```

The primary is tried; on error, alternates run in declared order
until one succeeds. The error returned is the *last* alternate's
error if all fail.

### Multi-provider fallbacks via `provider-*` features

Enable two or more provider features on the umbrella to tier across
runtimes — useful when one provider is rate-limited or down:

```toml
atomr-agents = { version = "0.2", features = ["agent", "provider-anthropic", "provider-openai"] }
```

```rust
use std::sync::Arc;
use atomr_agents::agent::{InferenceClient, LocalRunnerClient, Provider};
use atomr_agents::agent::providers::{anthropic, openai};
use atomr_agents::callable::with_fallbacks;

let primary: Arc<dyn InferenceClient> = Arc::new(LocalRunnerClient::new(
    anthropic::AnthropicRunner::new(anthropic::AnthropicConfig::from_env()?),
    Provider::Anthropic,
));
let backup: Arc<dyn InferenceClient> = Arc::new(LocalRunnerClient::new(
    openai::OpenAiRunner::new(openai::OpenAiConfig::from_env()?),
    Provider::OpenAi,
));

// Wrap each runner in its own pipeline stage and use with_fallbacks
// at the callable layer for tiered failover.
```

## Config tags + run name

```rust
use atomr_agents::callable::{with_config, RunConfig};

let traced = with_config(
    chain,
    RunConfig {
        run_name: Some("checkout-flow".into()),
        tags: vec!["v2".into(), "prod".into()],
        metadata: Default::default(),
    },
);
```

`run_name` and tags are pushed into `CallCtx::trace` (visible to
inner stages and to any tracer consuming the event stream).

## Timeouts

```rust
use atomr_agents::callable::with_timeout;
use std::time::Duration;

let bounded = with_timeout(slow_chain, Duration::from_secs(10));
```

Returns `AgentError::Internal("timed out after …")` if the inner
call doesn't finish in time. Pair with `with_retry` for "retry up to
N times, each capped at K seconds".

## Branch

```rust
use atomr_agents::callable::Branch;

let routed = Arc::new(Branch::new(
    |v: &Value| v.as_i64().unwrap_or(0) > 10,
    expensive_path,
    cheap_path,
));
```

`Branch` is itself a `Callable` so it slots into a pipeline.

## Lambda

`Lambda<F>` is a type alias for `FnCallable<F>` — the same closure
adapter, exposed under the LangChain-familiar name:

```rust
use atomr_agents::callable::Lambda;

let upper: CallableHandle = Arc::new(Lambda::labeled(
    "upper",
    |v: Value, _ctx| async move {
        let s = v.as_str().unwrap_or("").to_uppercase();
        Ok(Value::String(s))
    },
));
```

## Order of decorator application

From the inside out:

```rust
let final_chain =
    with_config(
        with_timeout(
            with_retry(
                with_fallbacks(primary, vec![alt]),
                RetryPolicy::default(),
            ),
            Duration::from_secs(30),
        ),
        RunConfig { run_name: Some("checkout".into()), ..Default::default() },
    );
```

Layered semantics:

1. `with_config` adds run name / tags to the trace.
2. `with_timeout` enforces wall-clock cap on the whole stack.
3. `with_retry` retries the next inner stack on error.
4. `with_fallbacks` falls back to alternates within each retry attempt.

## Canonical references

- [`docs/agent-pipeline.md`](https://github.com/rustakka/atomr-agents/blob/main/docs/agent-pipeline.md)
- [`docs/migrating-from-langgraph.md`](https://github.com/rustakka/atomr-agents/blob/main/docs/migrating-from-langgraph.md) — LCEL → Pipeline mapping
- [`crates/callable/src/pipeline.rs`](https://github.com/rustakka/atomr-agents/blob/main/crates/callable/src/pipeline.rs)
- [`crates/callable/src/decorators.rs`](https://github.com/rustakka/atomr-agents/blob/main/crates/callable/src/decorators.rs)

## Common mistakes

- **Building a pipeline once and discarding handles.** Pipelines
  hold `Arc`s; rebuilding on every call discards the work and may
  break upstream sharing.
- **Wrapping with `with_retry` something that's not idempotent.** If
  the operation has side effects (e.g. tool calls that charge a
  card), idempotency-key inside the operation, not retry around it.
- **Mixing `with_timeout(c, …)` with `with_retry(c, …)` in the
  wrong order.** `with_retry(with_timeout(c, ...), ...)` retries
  each timed call separately; `with_timeout(with_retry(c, ...),
  ...)` caps the *total* including retries.
- **Using `fan_out` with non-Send branches.** Each branch runs in a
  `tokio::spawn`; non-`Send` data won't compile.
