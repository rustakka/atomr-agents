//! Dynamic interrupts + static breakpoints + resume.
//!
//! `Interruptible` wraps a `StatefulRunner`-style execution with the
//! ability to:
//!
//! 1. Pause when a step calls `Interrupt::raise(payload)`.
//! 2. Pause before/after named steps via `interrupt_before` /
//!    `interrupt_after`.
//! 3. Persist the pause state as a checkpoint with a special label so
//!    the caller can `resume(run_id, command)` and continue.
//!
//! The pause mechanism is cooperative: a step runs `interrupt(...)`
//! and the runner translates that into a `RunOutcome::Paused`. Resume
//! re-enters the loop with the supplied `Command`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use atomr_agents_core::{AgentError, Result, RunId, Value, WorkflowId};
use atomr_agents_state::{CheckpointKey, Checkpointer, RunState, Snapshot, StateSchema};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::dag::{Dag, StepId};
use crate::state_runner::StatefulStep;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    /// Resume with no edits / no injected value.
    Continue,
    /// Resume; the supplied value is the return value of the
    /// `interrupt(...)` call inside the paused step.
    Resume(Value),
    /// Edit channels then continue.
    Update(Vec<(String, Value)>),
    /// Jump to a specific step on the next super-step.
    Goto(StepId),
}

#[derive(Debug, Clone)]
pub enum RunOutcome {
    /// Run completed normally.
    Done(RunState),
    /// Run paused; supply a `Command` to `Interruptible::resume`.
    Paused {
        super_step: u64,
        reason: PauseReason,
        payload: Option<Value>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PauseReason {
    DynamicInterrupt { step_id: StepId },
    Before(StepId),
    After(StepId),
}

/// Per-run interrupt control passed to a step. A step calls
/// `ctrl.interrupt(payload)` to pause; on resume the runner returns
/// the value from `Command::Resume(...)`.
#[derive(Clone)]
pub struct InterruptCtrl {
    inner: Arc<Mutex<Option<InterruptRequest>>>,
    /// On resume, the runner pre-populates this with the value from
    /// `Command::Resume(...)` so a step can read it.
    resume_value: Arc<Mutex<Option<Value>>>,
}

#[derive(Clone)]
struct InterruptRequest {
    step_id: StepId,
    payload: Option<Value>,
}

impl InterruptCtrl {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            resume_value: Arc::new(Mutex::new(None)),
        }
    }

    /// Called by step code to request a pause.
    pub fn interrupt(&self, step_id: StepId, payload: Option<Value>) {
        *self.inner.lock() = Some(InterruptRequest { step_id, payload });
    }

    /// Called by step code on the resume path to read the resume value.
    pub fn take_resume_value(&self) -> Option<Value> {
        self.resume_value.lock().take()
    }

    fn pending(&self) -> Option<InterruptRequest> {
        self.inner.lock().take()
    }

    fn set_resume_value(&self, v: Option<Value>) {
        *self.resume_value.lock() = v;
    }
}

impl Default for InterruptCtrl {
    fn default() -> Self {
        Self::new()
    }
}

/// `StatefulStep` extension that gets the interrupt ctrl as well.
#[async_trait::async_trait]
pub trait InterruptibleStep: Send + Sync + 'static {
    async fn run(&self, state: &RunState, ctrl: &InterruptCtrl) -> Result<Vec<(String, Value)>>;
}

/// Adapter that turns any `StatefulStep` into an `InterruptibleStep`.
pub struct PlainStep(pub Arc<dyn StatefulStep>);

#[async_trait::async_trait]
impl InterruptibleStep for PlainStep {
    async fn run(&self, state: &RunState, _ctrl: &InterruptCtrl) -> Result<Vec<(String, Value)>> {
        self.0.run(state).await
    }
}

/// Closure-friendly InterruptibleStep.
pub struct FnInterruptStep<F>(pub F);

#[async_trait::async_trait]
impl<F, Fut> InterruptibleStep for FnInterruptStep<F>
where
    F: Fn(&RunState, &InterruptCtrl) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<Vec<(String, Value)>>> + Send + 'static,
{
    async fn run(&self, state: &RunState, ctrl: &InterruptCtrl) -> Result<Vec<(String, Value)>> {
        (self.0)(state, ctrl).await
    }
}

pub struct Interruptible {
    pub workflow_id: WorkflowId,
    pub run_id: RunId,
    pub dag: Dag<Arc<dyn InterruptibleStep>>,
    pub schema: Arc<StateSchema>,
    pub checkpointer: Arc<dyn Checkpointer>,
    pub interrupt_before: HashSet<StepId>,
    pub interrupt_after: HashSet<StepId>,
}

impl Interruptible {
    pub async fn run(&self) -> Result<RunOutcome> {
        let snap = self.checkpointer.latest(&self.workflow_id, &self.run_id).await?;
        let mut state = match &snap {
            Some(s) => RunState::from_snapshot(self.schema.clone(), s.values.clone(), s.key.super_step),
            None => RunState::new(self.schema.clone()),
        };
        self.run_inner(&mut state, None, None, false).await
    }

    /// Resume from the most recent paused checkpoint.
    pub async fn resume(&self, command: Command) -> Result<RunOutcome> {
        let snap = self
            .checkpointer
            .latest(&self.workflow_id, &self.run_id)
            .await?
            .ok_or_else(|| AgentError::Workflow("resume: no checkpoint".into()))?;
        let (resume_value, edits, goto): (Option<Value>, Vec<(String, Value)>, Option<StepId>) = match command
        {
            Command::Continue => (None, Vec::new(), None),
            Command::Resume(v) => (Some(v), Vec::new(), None),
            Command::Update(es) => (None, es, None),
            Command::Goto(s) => (None, Vec::new(), Some(s)),
        };
        let mut values = snap.values.clone();
        for (k, v) in &edits {
            values.insert(k.clone(), v.clone());
        }
        let mut state = RunState::from_snapshot(self.schema.clone(), values, snap.key.super_step);
        // Resume always disables the next breakpoint hit so paused
        // breakpoints don't re-fire immediately. Dynamic interrupts
        // are similarly cleared by the snapshot label distinguishing
        // them.
        self.run_inner(&mut state, resume_value, goto, true).await
    }

    async fn run_inner(
        &self,
        state: &mut RunState,
        resume_value: Option<Value>,
        goto: Option<StepId>,
        mut skip_breakpoints_once: bool,
    ) -> Result<RunOutcome> {
        let order = self.dag.topo_sort()?;
        let layers = layered(&self.dag, &order);
        let resume_at = state.super_step();
        let ctrl = InterruptCtrl::new();
        let mut resume_value = resume_value;

        // Optionally jump to a layer containing `goto`.
        let goto_layer = goto.as_ref().and_then(|sid| {
            layers
                .iter()
                .position(|layer| layer.contains(sid))
                .map(|p| p as u64)
        });
        let start_layer = goto_layer.unwrap_or(resume_at);

        for (layer_idx, layer) in layers.iter().enumerate() {
            let super_step = layer_idx as u64 + 1;
            if super_step <= start_layer {
                continue;
            }
            // interrupt_before
            for sid in layer {
                if self.interrupt_before.contains(sid) {
                    if skip_breakpoints_once {
                        skip_breakpoints_once = false;
                        continue;
                    }
                    self.persist_pause(
                        state,
                        super_step.saturating_sub(1),
                        PauseReason::Before(sid.clone()),
                        None,
                    )
                    .await?;
                    return Ok(RunOutcome::Paused {
                        super_step,
                        reason: PauseReason::Before(sid.clone()),
                        payload: None,
                    });
                }
            }
            // Run all steps in the layer (sequential here so the
            // `ctrl.interrupt` semantics are unambiguous; full parallel
            // dispatch lands in R7).
            let mut all_writes: Vec<(String, Value)> = Vec::new();
            for sid in layer {
                if let Some(rv) = resume_value.take() {
                    ctrl.set_resume_value(Some(rv));
                }
                let step = self
                    .dag
                    .steps
                    .get(sid)
                    .ok_or_else(|| AgentError::Workflow(format!("missing step {}", sid.as_str())))?;
                let writes = step.run(state, &ctrl).await?;
                if let Some(req) = ctrl.pending() {
                    self.persist_pause(
                        state,
                        super_step.saturating_sub(1),
                        PauseReason::DynamicInterrupt {
                            step_id: req.step_id.clone(),
                        },
                        req.payload.clone(),
                    )
                    .await?;
                    return Ok(RunOutcome::Paused {
                        super_step,
                        reason: PauseReason::DynamicInterrupt {
                            step_id: req.step_id.clone(),
                        },
                        payload: req.payload,
                    });
                }
                all_writes.extend(writes);
            }
            state.merge_writes(all_writes)?;
            state.advance();
            self.checkpointer
                .save(Snapshot {
                    key: CheckpointKey {
                        workflow_id: self.workflow_id.clone(),
                        run_id: self.run_id.clone(),
                        super_step,
                    },
                    values: state.snapshot(),
                    label: format!("layer:{super_step}"),
                    timestamp_ms: now_ms(),
                })
                .await?;
            // interrupt_after
            for sid in layer {
                if self.interrupt_after.contains(sid) {
                    if skip_breakpoints_once {
                        skip_breakpoints_once = false;
                        continue;
                    }
                    return Ok(RunOutcome::Paused {
                        super_step,
                        reason: PauseReason::After(sid.clone()),
                        payload: None,
                    });
                }
            }
        }
        Ok(RunOutcome::Done(state.clone()))
    }

    async fn persist_pause(
        &self,
        state: &RunState,
        super_step: u64,
        reason: PauseReason,
        payload: Option<Value>,
    ) -> Result<()> {
        let label = match &reason {
            PauseReason::DynamicInterrupt { step_id } => {
                format!("interrupt:{}", step_id.as_str())
            }
            PauseReason::Before(s) => format!("before:{}", s.as_str()),
            PauseReason::After(s) => format!("after:{}", s.as_str()),
        };
        let mut values = state.snapshot();
        if let Some(p) = payload {
            values.insert("__interrupt_payload__".into(), p);
        }
        self.checkpointer
            .save(Snapshot {
                key: CheckpointKey {
                    workflow_id: self.workflow_id.clone(),
                    run_id: self.run_id.clone(),
                    super_step,
                },
                values,
                label,
                timestamp_ms: now_ms(),
            })
            .await
    }
}

fn layered(dag: &Dag<Arc<dyn InterruptibleStep>>, order: &[StepId]) -> Vec<Vec<StepId>> {
    let mut depth: HashMap<StepId, usize> = HashMap::new();
    for s in order {
        depth.insert(s.clone(), 0);
    }
    for s in order {
        if let Some(succs) = dag.edges.get(s) {
            let cur = depth[s];
            for n in succs {
                let next = (cur + 1).max(*depth.get(n).unwrap_or(&0));
                depth.insert(n.clone(), next);
            }
        }
    }
    let max_d = depth.values().copied().max().unwrap_or(0);
    let mut layers: Vec<Vec<StepId>> = vec![Vec::new(); max_d + 1];
    for s in order {
        layers[depth[s]].push(s.clone());
    }
    layers
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_state::{InMemoryCheckpointer, LastWriteWins, MergeMap, StateSchema};
    use serde_json::json;

    fn schema() -> Arc<StateSchema> {
        Arc::new(
            StateSchema::builder()
                .add("approved", LastWriteWins)
                .add("amount", LastWriteWins)
                .add("config", MergeMap)
                .build(),
        )
    }

    #[tokio::test]
    async fn dynamic_interrupt_pauses_then_resume_with_value() {
        let dag: Dag<Arc<dyn InterruptibleStep>> = Dag::builder("a")
            .step(
                "a",
                Arc::new(FnInterruptStep(|_state: &RunState, ctrl: &InterruptCtrl| {
                    let ctrl = ctrl.clone();
                    async move {
                        if let Some(approval) = ctrl.take_resume_value() {
                            // resumed
                            return Ok(vec![("approved".into(), approval)]);
                        }
                        ctrl.interrupt(StepId::new("a"), Some(json!({"q": "approve?"})));
                        Ok(vec![])
                    }
                })) as Arc<dyn InterruptibleStep>,
            )
            .step(
                "b",
                Arc::new(FnInterruptStep(|state: &RunState, _ctrl: &InterruptCtrl| {
                    let approved = state.read("approved").as_bool().unwrap_or(false);
                    async move { Ok(vec![("amount".into(), json!(if approved { 100 } else { 0 }))]) }
                })) as Arc<dyn InterruptibleStep>,
            )
            .edge("a", "b")
            .build();
        let cpt: Arc<dyn Checkpointer> = Arc::new(InMemoryCheckpointer::new());
        let r = Interruptible {
            workflow_id: WorkflowId::from("wf"),
            run_id: RunId::from("r"),
            dag,
            schema: schema(),
            checkpointer: cpt.clone(),
            interrupt_before: HashSet::new(),
            interrupt_after: HashSet::new(),
        };
        let out = r.run().await.unwrap();
        match out {
            RunOutcome::Paused { reason, payload, .. } => {
                assert!(matches!(reason, PauseReason::DynamicInterrupt { .. }));
                assert_eq!(payload.unwrap()["q"], "approve?");
            }
            _ => panic!("expected pause"),
        }
        let resumed = r.resume(Command::Resume(json!(true))).await.unwrap();
        match resumed {
            RunOutcome::Done(state) => {
                assert_eq!(state.read("approved"), &json!(true));
                assert_eq!(state.read("amount"), &json!(100));
            }
            _ => panic!("expected done"),
        }
    }

    #[tokio::test]
    async fn static_interrupt_before_pauses() {
        let mk_step = || -> Arc<dyn InterruptibleStep> {
            Arc::new(FnInterruptStep(|_s: &RunState, _c: &InterruptCtrl| async {
                Ok(vec![("config".into(), json!({"x": 1}))])
            }))
        };
        let dag: Dag<Arc<dyn InterruptibleStep>> = Dag::builder("a")
            .step("a", mk_step())
            .step("b", mk_step())
            .edge("a", "b")
            .build();
        let cpt: Arc<dyn Checkpointer> = Arc::new(InMemoryCheckpointer::new());
        let mut before = HashSet::new();
        before.insert(StepId::new("b"));
        let r = Interruptible {
            workflow_id: WorkflowId::from("wf"),
            run_id: RunId::from("r"),
            dag,
            schema: schema(),
            checkpointer: cpt.clone(),
            interrupt_before: before,
            interrupt_after: HashSet::new(),
        };
        let out = r.run().await.unwrap();
        match out {
            RunOutcome::Paused { reason, .. } => {
                assert!(matches!(reason, PauseReason::Before(s) if s.as_str() == "b"));
            }
            _ => panic!("expected pause before b"),
        }
        let done = r.resume(Command::Continue).await.unwrap();
        assert!(matches!(done, RunOutcome::Done(_)));
    }

    #[tokio::test]
    async fn update_command_edits_state_at_resume() {
        let dag: Dag<Arc<dyn InterruptibleStep>> = Dag::builder("only")
            .step(
                "only",
                Arc::new(FnInterruptStep(|state: &RunState, _c: &InterruptCtrl| {
                    let v = state.read("config").clone();
                    async move { Ok(vec![("config".into(), v)]) }
                })) as Arc<dyn InterruptibleStep>,
            )
            .build();
        let cpt: Arc<dyn Checkpointer> = Arc::new(InMemoryCheckpointer::new());
        let mut before = HashSet::new();
        before.insert(StepId::new("only"));
        let r = Interruptible {
            workflow_id: WorkflowId::from("wf"),
            run_id: RunId::from("r"),
            dag,
            schema: schema(),
            checkpointer: cpt,
            interrupt_before: before,
            interrupt_after: HashSet::new(),
        };
        let _ = r.run().await.unwrap();
        let done = r
            .resume(Command::Update(vec![(
                "config".into(),
                json!({"injected": true}),
            )]))
            .await
            .unwrap();
        match done {
            RunOutcome::Done(state) => {
                assert_eq!(state.read("config")["injected"], true);
            }
            _ => panic!("expected done"),
        }
    }
}
