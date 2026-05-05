# Workflows and human-in-the-loop

How atomr-agents wires deterministic graph execution, dynamic
fan-out, and human-in-the-loop pause/resume on top of channelled
state.

## Workflow runners

There are two runners in `agents-workflow`:

| Runner | When |
|---|---|
| `WorkflowRunner` | legacy `Step` enum (Invoke / Branch / Parallel / Loop / Map / Human); per-step output is opaque `Value` |
| `StatefulRunner` | channelled state via `StateSchema` + per-super-step `Checkpointer` snapshots |
| `Interruptible` | `StatefulRunner` + dynamic `interrupt()` + static breakpoints + resume API |

For new code, prefer `StatefulRunner` (or `Interruptible` if you
need HITL). The legacy `WorkflowRunner` exists for callers who want
DAG control flow without the channel/checkpoint discipline.

## DAG primitives

A `Dag<S>` is `BTreeMap<StepId, S>` plus an adjacency list. Build
with `Dag::builder(entry).step(...).edge(...).build()`. Topological
sort errors on cycles.

The `StatefulRunner` groups steps into super-steps by topological
*level* — all steps with the same in-degree-zero distance run in
parallel within a single super-step, then writes from all of them
merge through the channel reducers before the next super-step
starts.

```rust
use std::sync::Arc;
use atomr_agents_workflow::{Dag, FnStatefulStep, StatefulRunner, StatefulStep};

let dag: Dag<Arc<dyn StatefulStep>> = Dag::builder("a")
    .step("a", Arc::new(FnStatefulStep(|_s| async {
        Ok(vec![("messages".into(), serde_json::json!([{"id": "m1"}]))])
    })))
    .step("b1", Arc::new(FnStatefulStep(|_s| async {
        Ok(vec![("notes".into(), serde_json::json!({"path": "b1"}))])
    })))
    .step("b2", Arc::new(FnStatefulStep(|_s| async {
        Ok(vec![("notes".into(), serde_json::json!({"path": "b2"}))])
    })))
    .step("c", Arc::new(FnStatefulStep(|s| {
        let n = s.read("notes").as_object().map(|m| m.len()).unwrap_or(0);
        async move { Ok(vec![("phase".into(), serde_json::json!(format!("done-{n}")))]) }
    })))
    .edge("a", "b1")
    .edge("a", "b2")
    .edge("b1", "c")
    .edge("b2", "c")
    .build();

// b1 and b2 run concurrently in super-step 2; their writes merge via MergeMap
// into `notes`; c sees both before running in super-step 3.
```

## Send-API: dynamic fan-out

When the size of the fan-out is only known at runtime (e.g. "process
each item from the search result"), use `dispatch_fan_out`:

```rust
use atomr_agents_workflow::dispatch_fan_out;

let producer: CallableHandle = /* returns a Vec<Value> at runtime */;
let target:   CallableHandle = /* runs once per element */;

let outputs: Vec<Value> = dispatch_fan_out(
    producer,
    target,
    /* concurrency = */ 4,
    seed_input,
    ctx,
).await?;
// outputs.len() == producer.call(seed_input).await?.as_array().len()
// Order preserved.
```

This is the atomr-agents analogue of LangGraph's `Send` API.

## Subgraphs

A `Subgraph` packages a `StatefulRunner`-style execution as a
`Callable` so a parent workflow can call it as a step. The parent
declares input/output channel projections:

```rust
use atomr_agents_workflow::Subgraph;

let sub = Subgraph {
    workflow_id: "child".into(),
    run_id: "child-run".into(),
    dag: child_dag,
    schema: child_schema,
    checkpointer: cpt.clone(),
    input_channels:  vec!["messages".into()],   // read from parent state
    output_channels: vec!["notes".into()],      // merged back into parent
};

// Subgraph is Callable. Wrap it in any pipeline / workflow step.
let result = sub.call(parent_input, ctx).await?;
// result.outputs.notes  is the projected output
// result.private_state  is the full child snapshot (handy for debugging)
```

Channels not in `input_channels` or `output_channels` are private to
the child — the parent never sees them. This is how multi-agent
"specialist" subgraphs maintain their own working memory without
leaking it into shared state.

## Dynamic interrupts

A step can pause execution mid-way by calling `ctrl.interrupt(...)`:

```rust
use atomr_agents_workflow::{
    Command, FnInterruptStep, InterruptCtrl, Interruptible, InterruptibleStep, RunOutcome,
};
use std::collections::HashSet;

let step: Arc<dyn InterruptibleStep> = Arc::new(FnInterruptStep(
    |state: &RunState, ctrl: &InterruptCtrl| {
        let ctrl = ctrl.clone();
        async move {
            // On resume, take_resume_value returns the Command::Resume(...) payload.
            if let Some(approval) = ctrl.take_resume_value() {
                return Ok(vec![("approved".into(), approval)]);
            }
            // Otherwise, request a pause and let the caller resume us.
            ctrl.interrupt("approval-step".into(), Some(serde_json::json!({
                "question": "Approve $5,000 transfer?"
            })));
            Ok(vec![])
        }
    },
));
```

The runner persists a special checkpoint with label
`interrupt:approval-step`, captures the payload under a reserved
state key (`__interrupt_payload__`), and returns
`RunOutcome::Paused`.

## Static breakpoints

For step-debugging or approval gates that don't need the step itself
to know about the pause, set `interrupt_before` / `interrupt_after`
when constructing the `Interruptible`:

```rust
let mut before = HashSet::new();
before.insert(StepId::new("spend_money"));

let r = Interruptible {
    workflow_id: "wf".into(),
    run_id: "run".into(),
    dag,
    schema,
    checkpointer: cpt,
    interrupt_before: before,
    interrupt_after: HashSet::new(),
};
```

The runner pauses *before* the named step's super-step, persisting a
`before:spend_money` checkpoint. `interrupt_after` pauses *after*
(useful for "review the result before continuing").

## The Command resume API

```rust
pub enum Command {
    Continue,                          // resume with no edits
    Resume(Value),                     // resume; value injected into the paused step's `take_resume_value`
    Update(Vec<(String, Value)>),       // edit channels, then resume
    Goto(StepId),                       // jump to a specific step on resume
}
```

```rust
match r.run().await? {
    RunOutcome::Paused { reason, payload, .. } => {
        // Show payload.question to a human, get approval.
        let approved = serde_json::json!(true);
        let done = r.resume(Command::Resume(approved)).await?;
        // …or edit state then continue:
        // r.resume(Command::Update(vec![("config".into(), serde_json::json!(...))])).await?;
        // …or jump:
        // r.resume(Command::Goto(StepId::new("retry_branch"))).await?;
    }
    RunOutcome::Done(state) => { /* normal exit */ }
}
```

Resume always disables the next breakpoint hit so you don't
infinite-loop on a static `Before` breakpoint.

## Approval / edit / reject patterns

These are all `Command` selections wrapped in UX:

| Pattern | Selection |
|---|---|
| Approve | `Command::Continue` (no edits) |
| Approve with payload | `Command::Resume(value)` |
| Edit + continue | `Command::Update(vec![(key, value), …])` |
| Reject + retry from earlier | `Command::Goto(retry_step)` after `Command::Update` to reset state |

## Where to go from here

- [State and checkpointing](state-and-checkpointing.md) — channels,
  reducers, `Checkpointer::fork` underpinning all of the above.
- [Multi-agent patterns](multi-agent-patterns.md) — supervisor pauses
  to confirm a handoff; swarm handoff tools return a `Command`.
- [Eval](eval.md) — replay-based regression eval over recorded
  workflow runs.
