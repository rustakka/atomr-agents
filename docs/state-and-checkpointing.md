# State and checkpointing

How atomr-agents models long-running agent state — typed channels with
reducers, per-super-step snapshots, fork-with-edit, and resume after
crash.

## The state model

LangGraph has a load-bearing primitive: a graph state is a typed
object whose keys are *channels*, and each channel has a **reducer**
that defines how concurrent writes merge. atomr-agents ships the same
model verbatim, in atomr's idiom:

```rust
use std::sync::Arc;
use atomr_agents_state::{
    AppendMessages, InMemoryCheckpointer, MergeMap, RunState, StateSchema,
};

let schema = Arc::new(
    StateSchema::builder()
        .add("messages", AppendMessages)   // append-with-id-dedup
        .add("config",   MergeMap)         // shallow object merge
        .add_lww("phase")                   // last-write-wins
        .build(),
);

let mut state = RunState::new(schema);
state.write("messages", serde_json::json!([{"id": "m1", "text": "hi"}]))?;
state.write("messages", serde_json::json!([{"id": "m1", "text": "edit"}]))?;
// AppendMessages dedupes by id: state["messages"][0].text == "edit"
```

## The five reducers

| Reducer | Use case |
|---|---|
| `LastWriteWins` | scalar config, current step id, phase markers |
| `AppendList` | event log, observation list, audit trail |
| `AppendMessages` | chat history with idempotent retry — appends new ids, replaces same id |
| `MergeMap` | structured config, partial settings, accumulated metadata |
| `MaxByTimestamp` | sensor / heartbeat values where the newer write wins by `ts_ms` |

Custom reducers are one trait impl:

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

Reducers must be **associative** so parallel writes within a single
super-step can be folded in any order without losing data. They do
not need to be commutative.

## RunState — the runtime container

`RunState` holds the channelled values plus the current super-step
counter. `write` routes through the channel's reducer; `merge_writes`
applies a batch (typically all writes from one super-step, including
any from parallel branches in the DAG):

```rust
state.merge_writes(vec![
    ("messages".into(), serde_json::json!([{"id": "m1"}])),
    ("messages".into(), serde_json::json!([{"id": "m2"}])),
    ("config".into(),   serde_json::json!({"a": 1})),
])?;
state.advance();  // bump super_step
```

Unknown channels error — schema is the source of truth.

## Checkpointer

```rust
#[async_trait]
pub trait Checkpointer: Send + Sync + 'static {
    async fn save(&self, snapshot: Snapshot) -> Result<()>;
    async fn load(&self, key: &CheckpointKey) -> Result<Option<Snapshot>>;
    async fn latest(&self, workflow: &WorkflowId, run: &RunId) -> Result<Option<Snapshot>>;
    async fn list(&self, workflow: &WorkflowId, run: &RunId) -> Result<Vec<CheckpointMeta>>;
    async fn fork(&self, from: &CheckpointKey, edits: Vec<(String, Value)>) -> Result<RunId>;
}
```

A `CheckpointKey` is `(workflow_id, run_id, super_step)`. `Snapshot`
holds the full `values: HashMap<String, Value>`, a `label`, and a
timestamp.

`InMemoryCheckpointer` is the default. SQLite + Postgres backends
ship as feature-gated stubs; real wiring lives in deployment
patches:

```toml
atomr-agents-state = { version = "0.1", features = ["sqlite"] }
```

```rust
let cpt: Arc<dyn Checkpointer> = Arc::new(
    SqliteCheckpointer::connect("sqlite://./checkpoints.db").await?,
);
```

## StatefulRunner — channelled DAG execution

`StatefulRunner` runs a `Dag<Arc<dyn StatefulStep>>` layer-by-layer
(one super-step per topological layer), persists a snapshot after
every layer, and resumes from the latest checkpoint on restart:

```rust
use atomr_agents_workflow::{Dag, StatefulRunner, StatefulStep, FnStatefulStep};

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
    workflow_id: "wf-1".into(),
    run_id: "run-1".into(),
    dag,
    schema,
    checkpointer: cpt,
};
let final_state = runner.run().await?;
```

If the process dies mid-execution and restarts with the same
`(workflow_id, run_id)`, the runner reads the latest checkpoint,
hydrates `RunState` from it, and resumes from the next un-completed
super-step. Already-completed steps **are not re-executed** —
side-effecting steps (counter increments, external API calls) only
fire once.

## Fork with edit

The `fork` operation is what makes time-travel debugging and HITL
approval flows tractable:

```rust
// Create a divergent run from super_step 2 with an edited config.
let new_run_id = cpt.fork(
    &CheckpointKey {
        workflow_id: "wf-1".into(),
        run_id: "run-1".into(),
        super_step: 2,
    },
    vec![("config".into(), serde_json::json!({"feature_x": true}))],
).await?;

// new_run_id picks up where run-1 was at step 2, but with the edit
// applied. Both runs continue independently from there.
```

`fork` is the substrate underneath `Command::Update` (state edit
during HITL resume) and the `POST /runs/:id/fork` Studio endpoint.

## Where to go from here

- [Workflows and HITL](workflows-and-hitl.md) — `Interruptible`,
  static breakpoints, `Command::{Continue, Resume, Update, Goto}`,
  resume API.
- [Multi-agent patterns](multi-agent-patterns.md) — channelled state
  shared across agents (e.g. `ActiveAgent` slot for swarm handoff).
- [Architecture](architecture.md) — where the state layer fits in
  the crate stack.
