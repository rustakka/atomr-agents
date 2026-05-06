---
name: atomr-agents-middleware
description: Use when wrapping the agent's per-turn pipeline with cross-cutting policies — logging, retry, rate-limit, redaction, tool-error recovery, dynamic prompt override, before/after hooks. Triggers on `impl AgentMiddleware for`, `MiddlewareStack::new().push(...)`, or porting a LangChain `create_agent` middleware (`@wrap_model_call` / `@wrap_tool_call` / `@dynamic_prompt`).
---

# Agent middleware in atomr-agents

`AgentMiddleware` wraps the agent's per-turn pipeline with optional
hooks for each layer. This is atomr-agents' answer to LangChain
1.x's `create_agent` middleware system.

## Mental model

- Middleware is a trait with **defaultable async methods**, one per
  pipeline phase (`before_agent`, `before_model_call`,
  `after_model_call`, `before_tool_call`, `after_tool_call`,
  `after_agent`, `dynamic_prompt`).
- A `MiddlewareStack` holds multiple middlewares. The agent runs
  `before_*` hooks in **registration order** and `after_*` hooks in
  **reverse order** — the standard Tower convention.
- Each hook can mutate the value it receives via `&mut`. A
  `Some(_)` from `dynamic_prompt` overrides the system prompt for
  this turn (last middleware wins).

## The trait

```rust
#[async_trait::async_trait]
pub trait AgentMiddleware: Send + Sync + 'static {
    async fn before_agent(&self, _agent_id: &AgentId, _user: &str) -> Result<()> { Ok(()) }
    async fn before_model_call(&self, _batch: &mut ExecuteBatch) -> Result<()> { Ok(()) }
    async fn after_model_call(&self, _result: &mut TurnResult) -> Result<()> { Ok(()) }
    async fn before_tool_call(&self, _name: &str, _args: &mut Value) -> Result<()> { Ok(()) }
    async fn after_tool_call(&self, _name: &str, _result: &mut Result<Value>) -> Result<()> { Ok(()) }
    async fn after_agent(&self, _result: &mut TurnResult) -> Result<()> { Ok(()) }
    async fn dynamic_prompt(&self, _agent_id: &AgentId, _user: &str) -> Result<Option<String>> { Ok(None) }
}
```

Override only the hooks you need; defaults are no-ops.

## Building a stack

```rust
use std::sync::Arc;
use atomr_agents::agent::{
    AgentMiddleware, LoggingMiddleware, MiddlewareStack, RateLimitMiddleware,
    RedactionMiddleware, ToolErrorRecoveryMiddleware,
};

let stack = MiddlewareStack::new()
    .push(Arc::new(LoggingMiddleware::new()))
    .push(Arc::new(RateLimitMiddleware::new(/* capacity */ 10, /* refill_per_sec */ 5)))
    .push(Arc::new(RedactionMiddleware::new(
        vec!["secret_key=".into(), "api_token=".into()],
        "[redacted]",
    )))
    .push(Arc::new(ToolErrorRecoveryMiddleware));
```

Order matters:

- `before_agent`: Logging → RateLimit → Redaction → ToolErrorRecovery
- `before_model_call`: same order
- `after_model_call`: ToolErrorRecovery → Redaction → RateLimit → Logging
- `before_tool_call`: Logging → RateLimit → Redaction → ToolErrorRecovery
- `after_tool_call`: ToolErrorRecovery → Redaction → RateLimit → Logging
- `after_agent`: ToolErrorRecovery → Redaction → RateLimit → Logging

## Stock middlewares

### LoggingMiddleware

Captures one log line per phase. Useful for diagnostics; production
should plug into the EventBus / RunTreeBuilder instead.

```rust
let log = Arc::new(LoggingMiddleware::new());
// After running a turn:
println!("{:?}", log.lines());
```

### RateLimitMiddleware

Token-bucket gate on `before_model_call`. Per-agent or shared:

```rust
// 10 requests burst, refill 5/sec:
RateLimitMiddleware::new(10, 5)
```

Calls beyond the burst block (await) up to 10 seconds before
returning `AgentError::Inference("rate-limit: gave up after 10s")`.

### RedactionMiddleware

Strip patterns from outgoing user messages on `before_model_call`:

```rust
RedactionMiddleware::new(
    vec!["password=".into(), "api_key=".into()],
    "[redacted]",
)
```

For regex-based redaction, write a custom middleware (see below).

### ToolErrorRecoveryMiddleware

Convert tool errors into model-readable payloads on
`after_tool_call` so the model sees a structured error and can
recover instead of bubbling out:

```rust
ToolErrorRecoveryMiddleware
// {"tool_error": true, "tool": <name>, "message": <error string>}
```

## Inspecting the model's tool calls

`TurnResult.tool_calls: Vec<ParsedToolCall>` is populated with every
tool call the agent processed during the turn (aggregated across all
tool-call iterations, not just the final one). `after_model_call` and
`after_agent` can inspect or mutate it — useful for "max tools per
turn" caps, post-hoc auditing, or routing decisions:

```rust
use atomr_agents_core::AgentError;
use atomr_agents::agent::{AgentMiddleware, TurnResult};

pub struct MaxToolsGuard { pub max: usize }

#[async_trait]
impl AgentMiddleware for MaxToolsGuard {
    async fn after_model_call(&self, result: &mut TurnResult) -> Result<()> {
        if result.tool_calls.len() > self.max {
            return Err(AgentError::Tool(format!(
                "agent emitted {} tool calls, max is {}",
                result.tool_calls.len(), self.max,
            )));
        }
        Ok(())
    }
}
```

`TurnResult.usage` (a `TokenUsage`) also includes the new
`reasoning_tokens` and `cached_tokens` fields — use them in
cost-tracking middleware.

## Authoring a custom middleware

```rust
use async_trait::async_trait;
use std::sync::Arc;
use atomr_agents_core::{AgentId, Result};
use atomr_agents::agent::{AgentMiddleware, TurnResult};
use atomr_infer_core::batch::ExecuteBatch;

pub struct ModelOverrideMiddleware {
    pub prefer_for: Vec<String>,   // user-message substrings
    pub model: String,
}

#[async_trait]
impl AgentMiddleware for ModelOverrideMiddleware {
    async fn before_model_call(&self, batch: &mut ExecuteBatch) -> Result<()> {
        for m in &batch.messages {
            let text = match &m.content {
                atomr_infer_core::batch::MessageContent::Text(t) => t.clone(),
                _ => continue,
            };
            if self.prefer_for.iter().any(|p| text.contains(p)) {
                batch.model = self.model.clone();
                return Ok(());
            }
        }
        Ok(())
    }
}
```

Now `ModelOverrideMiddleware { prefer_for: vec!["analyze".into()],
model: "claude-3-opus".into() }` swaps to a more powerful model
when the prompt mentions analysis.

## Dynamic prompt override

```rust
struct PersonaSwitcher;

#[async_trait]
impl AgentMiddleware for PersonaSwitcher {
    async fn dynamic_prompt(&self, agent_id: &AgentId, user: &str) -> Result<Option<String>> {
        if user.starts_with("/teach") {
            Ok(Some("You are a patient teacher. Explain step by step.".into()))
        } else if user.starts_with("/code") {
            Ok(Some("You are a senior engineer. Reply with runnable code.".into()))
        } else {
            Ok(None)  // keep the agent's normal instruction strategy
        }
    }
}
```

Last middleware to return `Some(_)` wins.

## Wiring middleware to the agent

The `Agent` struct accepts an optional `middleware: MiddlewareStack`
field (or `Vec<Arc<dyn AgentMiddleware>>`). The agent's `run_turn`
dispatches each phase through the stack automatically.

For now (v0), the recommended pattern is:

```rust
// At the start of run_turn:
stack.run_before_agent(&self.id, &user).await?;

// Wrap model call:
stack.run_before_model_call(&mut batch).await?;
let mut r = self.inference.run(batch).await?;
stack.run_after_model_call(&mut r).await?;

// Wrap each tool call:
stack.run_before_tool_call(&call.name, &mut args).await?;
let mut result = tool_handle.call(args.clone(), invoke_ctx).await;
stack.run_after_tool_call(&call.name, &mut result).await?;
let result = result?;

// At the end:
stack.run_after_agent(&mut turn_result).await?;
```

## Canonical references

- [`docs/agent-pipeline.md`](https://github.com/rustakka/atomr-agents/blob/main/docs/agent-pipeline.md) — middleware in the per-turn pipeline
- [`crates/agent/src/middleware.rs`](https://github.com/rustakka/atomr-agents/blob/main/crates/agent/src/middleware.rs) — trait + stock implementations

## Common mistakes

- **Mutating fields the agent later reads back.** `&mut TurnResult`
  in `after_model_call` works, but downstream middleware in the
  same `after_*` chain sees the mutation. Plan the order.
- **Heavy work in `before_model_call`.** It runs inline before the
  inference call; long blocking pushes the model TTFB out.
- **Side-effecting `dynamic_prompt`.** Multiple middlewares race;
  order is registration. Side effects should live in
  `before_agent`.
- **Forgetting `Ok(())` on no-op overrides.** The default already
  returns `Ok(())`; explicit `?` propagation is fine but verbose.
- **`Sync` issues with custom middleware fields.** `AgentMiddleware`
  is `Send + Sync + 'static` — wrap mutable state in `Arc<Mutex>`
  or `parking_lot::RwLock`.
