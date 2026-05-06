---
name: atomr-agents-state
description: Use when designing or modifying channelled state — declaring a `StateSchema`, picking a reducer, persisting via `Checkpointer`, or forking a divergent run. Triggers on writing `StateSchema::builder()`, `RunState::new()`, `Checkpointer::save / latest / fork`, or porting a LangGraph `StateGraph(MyState)` definition.
---

# Channelled state in atomr-agents

`agents-state` is the framework's stateful-graph layer. State is a
typed map of channels; each channel has a reducer that merges
existing values with incoming writes; checkpoints persist after
every super-step so workflows resume cleanly after a crash and
operators can fork divergent runs.

## Mental model

- A **state schema** is a registry of channels.
- A **channel** is `(key, reducer, default)`.
- A **reducer** merges current value with incoming write — and is
  *associative*, so parallel writes within a single super-step can
  fold in any order.
- A **super-step** is a topological layer of the DAG. Within one
  super-step, all steps run concurrently; their writes merge through
  the reducers; the result is checkpointed; then the next layer
  starts.
- **Resume** rehydrates from the latest checkpoint and skips already-
  completed super-steps.
- **Fork** creates a divergent `RunId` from any prior checkpoint with
  optional state edits applied at the fork point.

## Picking a reducer

| Reducer | Picks when |
|---|---|
| `LastWriteWins` | scalar config / phase / current step pointer |
| `AppendList` | event log, observations, audit trail |
| `AppendMessages` | chat history with idempotent retry — appends new ids, replaces same id |
| `MergeMap` | structured config, partial settings, accumulated metadata |
| `MaxByTimestamp` | sensor / heartbeat values — newer write wins by `ts_ms` field |

Custom reducer:

```rust
use atomr_agents_state::Reducer;

struct Sum;
impl Reducer for Sum {
    fn reduce(&self, current: serde_json::Value, incoming: serde_json::Value) -> serde_json::Value {
        let a = current.as_i64().unwrap_or(0);
        let b = incoming.as_i64().unwrap_or(0);
        serde_json::json!(a + b)
    }
}
```

Reducers must be associative; they don't need to be commutative.

## Declaring a schema

```rust
use std::sync::Arc;
use atomr_agents_state::{
    AppendMessages, MaxByTimestamp, MergeMap, RunState, StateSchema,
};

let schema = Arc::new(
    StateSchema::builder()
        .add("messages", AppendMessages)
        .add("config",   MergeMap)
        .add_lww("phase")                            // shorthand for LastWriteWins
        .add("heartbeat", MaxByTimestamp)
        .add_with_default(
            "counter",
            crate::Sum,
            serde_json::json!(0),
        )
        .build(),
);
```

## Reading and writing state

```rust
let mut state = RunState::new(schema);

// Single write through the channel's reducer.
state.write("messages", serde_json::json!([{"id": "m1", "text": "hi"}]))?;

// Batch (one super-step's worth of writes).
state.merge_writes(vec![
    ("config".into(),   serde_json::json!({"feature_x": true})),
    ("messages".into(), serde_json::json!([{"id": "m2"}])),
])?;

// Reads.
let msgs = state.read("messages");
assert_eq!(state.super_step(), 0);  // hasn't been advance()d yet
```

`write` to an unknown channel errors — schema is the source of
truth.

## Checkpointing

```rust
use std::sync::Arc;
use atomr_agents_state::{Checkpointer, InMemoryCheckpointer};

let cpt: Arc<dyn Checkpointer> = Arc::new(InMemoryCheckpointer::new());
```

For production, use a backend feature flag:

```toml
[dependencies]
atomr-agents-state = { version = "0.2", features = ["sqlite"] }
# or "postgres"
```

```rust
use atomr_agents_state::SqliteCheckpointer;

let cpt: Arc<dyn Checkpointer> = Arc::new(
    SqliteCheckpointer::connect("sqlite://./checkpoints.db").await?,
);
```

(SQLite/Postgres ship as feature-gated stubs in the open-source
crate; wire to `sqlx` in your deployment patch.)

## Driving a stateful workflow

```rust
use atomr_agents_workflow::{Dag, FnStatefulStep, StatefulRunner, StatefulStep};
use atomr_agents_core::{RunId, WorkflowId};

let dag: Dag<Arc<dyn StatefulStep>> = Dag::builder("a")
    .step("a", Arc::new(FnStatefulStep(|_state| async {
        Ok(vec![("messages".into(), serde_json::json!([{"id": "m1"}]))])
    })))
    .step("b", Arc::new(FnStatefulStep(|state| {
        let n = state.read("messages").as_array().map(|v| v.len()).unwrap_or(0);
        async move { Ok(vec![("config".into(), serde_json::json!({"seen": n}))]) }
    })))
    .edge("a", "b")
    .build();

let runner = StatefulRunner {
    workflow_id: WorkflowId::from("wf-1"),
    run_id:      RunId::from("run-1"),
    dag,
    schema,
    checkpointer: cpt.clone(),
};
let final_state = runner.run().await?;
```

If the process dies and restarts with the same `(workflow_id,
run_id)`, the runner reads the latest checkpoint and resumes from
the next un-executed super-step. **Side-effecting steps (counter
increments, external API calls) only fire once.**

## Fork with edit

```rust
use atomr_agents_state::CheckpointKey;

let new_run = cpt.fork(
    &CheckpointKey {
        workflow_id: WorkflowId::from("wf-1"),
        run_id:      RunId::from("run-1"),
        super_step:  2,
    },
    vec![
        ("config".into(), serde_json::json!({"feature_x": true})),
    ],
).await?;
```

`fork` is the substrate for time-travel debugging, the
`POST /runs/:id/fork` Studio endpoint, and `Command::Update` during
HITL resume.

## Canonical references

- [`docs/state-and-checkpointing.md`](https://github.com/rustakka/atomr-agents/blob/main/docs/state-and-checkpointing.md)
- [`crates/state/src/reducer.rs`](https://github.com/rustakka/atomr-agents/blob/main/crates/state/src/reducer.rs)
- [`crates/state/src/checkpointer.rs`](https://github.com/rustakka/atomr-agents/blob/main/crates/state/src/checkpointer.rs)
- [`crates/workflow/src/state_runner.rs`](https://github.com/rustakka/atomr-agents/blob/main/crates/workflow/src/state_runner.rs)

## Common mistakes

- **Writing to a channel not in the schema.** Errors at runtime.
  Add the channel or pick the right key.
- **Using `LastWriteWins` for chat history.** Parallel branches will
  clobber each other. Use `AppendMessages`.
- **Non-associative custom reducer.** Parallel super-step writes
  fold in non-deterministic order; non-associative reducers produce
  non-deterministic results.
- **Different `RunId` on resume.** `(workflow_id, run_id)` keys the
  checkpoint. Different `RunId` = fresh run, no resume.
- **Forgetting `state.advance()` between super-steps.** Manual
  driving requires it; `StatefulRunner` does it for you.
- **Forking from a non-existent checkpoint.** `Checkpointer::fork`
  errors. Inspect with `list(workflow, run)` first.
