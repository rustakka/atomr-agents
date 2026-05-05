---
name: atomr-agents-hitl
description: Use when adding human-in-the-loop pause/resume — dynamic `interrupt()` from a step, static breakpoints (`interrupt_before` / `interrupt_after`), or driving resume with `Command::{Continue, Resume, Update, Goto}`. Triggers on writing `Interruptible { ... }`, calling `ctrl.interrupt(...)`, or porting a LangGraph `interrupt()` / `Command(resume=...)` flow.
---

# Human-in-the-loop in atomr-agents

`Interruptible` extends the stateful workflow runner with three
HITL primitives: dynamic `interrupt()` from inside a step, static
breakpoints on named steps, and a `Command` resume API. All three
ride on the `Checkpointer` from `agents-state`.

## Mental model

- **Dynamic interrupt.** A step calls
  `ctrl.interrupt(step_id, Some(payload))` and returns. The runner
  persists a special checkpoint labelled
  `interrupt:<step>` and returns `RunOutcome::Paused`.
- **Static breakpoints.** `Interruptible::interrupt_before` /
  `_after` is a `HashSet<StepId>`. Hitting one persists a
  `before:<step>` / `after:<step>` checkpoint and returns
  `RunOutcome::Paused`.
- **Resume.** Caller invokes `Interruptible::resume(command)`. On
  resume, the runner disables the next breakpoint hit so a static
  `Before` doesn't immediately re-fire.

## Dynamic interrupt

```rust
use std::sync::Arc;
use std::collections::HashSet;
use atomr_agents_workflow::{
    Command, FnInterruptStep, InterruptCtrl, Interruptible, InterruptibleStep, RunOutcome,
    Dag, StepId,
};
use atomr_agents_state::{InMemoryCheckpointer, LastWriteWins, RunState, StateSchema};
use atomr_agents_core::{RunId, WorkflowId};

let schema = Arc::new(
    StateSchema::builder()
        .add("approved", LastWriteWins)
        .build(),
);

let approve_step: Arc<dyn InterruptibleStep> = Arc::new(FnInterruptStep(
    |_state: &RunState, ctrl: &InterruptCtrl| {
        let ctrl = ctrl.clone();
        async move {
            // On resume, the value passed to Command::Resume(...) shows up here.
            if let Some(approval) = ctrl.take_resume_value() {
                return Ok(vec![("approved".into(), approval)]);
            }
            ctrl.interrupt(
                StepId::new("approve"),
                Some(serde_json::json!({"prompt": "Approve $5,000 transfer?"})),
            );
            Ok(vec![])  // returns empty; the runner persists the pause
        }
    },
));

let dag: Dag<Arc<dyn InterruptibleStep>> = Dag::builder("approve")
    .step("approve", approve_step)
    .build();

let runner = Interruptible {
    workflow_id: WorkflowId::from("wf"),
    run_id:      RunId::from("run-1"),
    dag,
    schema,
    checkpointer: Arc::new(InMemoryCheckpointer::new()),
    interrupt_before: HashSet::new(),
    interrupt_after:  HashSet::new(),
};

match runner.run().await? {
    RunOutcome::Paused { reason, payload, .. } => {
        // Show payload to a human, get their answer …
        let approved = serde_json::json!(true);
        let RunOutcome::Done(state) = runner.resume(Command::Resume(approved)).await? else {
            unreachable!();
        };
        assert_eq!(state.read("approved"), &serde_json::json!(true));
    }
    RunOutcome::Done(_) => unreachable!("we requested a pause"),
}
```

## Static breakpoints

For pauses that don't require the step to know about them
(step-debugging, approval gates):

```rust
let mut before = HashSet::new();
before.insert(StepId::new("spend_money"));

let runner = Interruptible {
    /* ... */
    interrupt_before: before,
    interrupt_after:  HashSet::new(),
};
```

The runner pauses *before* `spend_money` runs in its super-step.
`interrupt_after` pauses immediately after the named step
completes.

## The Command resume API

```rust
pub enum Command {
    Continue,                           // resume with no edits
    Resume(Value),                      // resume; injects value into ctrl.take_resume_value()
    Update(Vec<(String, Value)>),       // edit channels via reducers, then resume
    Goto(StepId),                       // jump to a specific step on resume
}
```

Use cases:

| UX | Command |
|---|---|
| Approve | `Command::Continue` |
| Approve with payload | `Command::Resume(serde_json::json!(true))` |
| Edit + continue | `Command::Update(vec![("config".into(), serde_json::json!(...))])` |
| Reject + retry | `Command::Update(vec![/* reset */])` followed by `Command::Goto(retry_step)` |

`Command::Update` writes go through the channel reducers (so e.g.
an `AppendMessages` channel still dedupes by id when you inject
new messages mid-flight).

## Resume disables next breakpoint

`Interruptible::resume(...)` always sets a one-shot
`skip_breakpoints_once` flag so a static `Before` breakpoint doesn't
immediately re-fire on resume. Without this, `Command::Continue`
would loop forever on the same breakpoint.

Dynamic interrupts don't loop because the step itself reads
`ctrl.take_resume_value()` and skips the `interrupt(...)` call on
the resume path (your step code is responsible for this — see the
`approve_step` example above).

## Combining with checkpointer fork

`Command::Update` performs an in-place edit. To create a divergent
run instead (so the original paused run is preserved), call
`Checkpointer::fork(checkpoint_key, edits)` directly, then run a
new `Interruptible` keyed on the new `RunId`:

```rust
let new_run = cpt.fork(
    &CheckpointKey {
        workflow_id: "wf".into(),
        run_id: "run-1".into(),
        super_step: 0,           // pause persisted at super_step before "approve"
    },
    vec![("approved".into(), serde_json::json!(false))],
).await?;

let alt_runner = Interruptible {
    run_id: new_run,
    /* same dag/schema/cpt */
    ..runner_template
};
let _ = alt_runner.run().await?;
```

## Canonical references

- [`docs/workflows-and-hitl.md`](https://github.com/rustakka/atomr-agents/blob/main/docs/workflows-and-hitl.md)
- [`docs/state-and-checkpointing.md`](https://github.com/rustakka/atomr-agents/blob/main/docs/state-and-checkpointing.md)
- [`crates/workflow/src/interrupt.rs`](https://github.com/rustakka/atomr-agents/blob/main/crates/workflow/src/interrupt.rs)

## Common mistakes

- **Forgetting `take_resume_value()` in the step.** Without it the
  step pauses, resumes, pauses again — infinite loop.
- **Using `Command::Continue` after a dynamic interrupt that
  expects a value.** The step's `take_resume_value()` returns
  `None`; it'll re-pause. Use `Command::Resume(Value::Null)` to
  resume without injecting data.
- **Resuming from a different `RunId`.** Resume re-reads from the
  checkpointer keyed on `(workflow_id, run_id)`. Wrong id → no
  checkpoint found → error.
- **Forgetting to disable static breakpoints when iterating.** They
  pause every time the corresponding super-step is reached. Drop
  them from the `interrupt_before`/`_after` sets when you're done
  debugging.
- **`Command::Goto` to a step in a finished super-step.** The
  runner skips already-completed super-steps. Use `fork` if you
  need to rewind.
