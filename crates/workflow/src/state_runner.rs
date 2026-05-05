//! Stateful runner — DAG over a `StateSchema` + `Checkpointer`.
//!
//! Each step is a `StatefulStep` that takes the current `RunState`,
//! returns a list of writes (channel-key, value), and the runner
//! applies them via the channel reducers. After every super-step
//! the snapshot is persisted via the `Checkpointer`. On resume,
//! `WorkflowRunner::resume_from_checkpoint` skips through completed
//! super-steps using the journal's existing `StepCompleted` events.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentError, Result, RunId, Value, WorkflowId};
use atomr_agents_state::{
    Checkpointer, CheckpointKey, RunState, Snapshot, StateSchema,
};

use crate::dag::{Dag, StepId};

#[async_trait]
pub trait StatefulStep: Send + Sync + 'static {
    async fn run(&self, state: &RunState) -> Result<Vec<(String, Value)>>;
}

pub struct StatefulRunner {
    pub workflow_id: WorkflowId,
    pub run_id: RunId,
    pub dag: Dag<Arc<dyn StatefulStep>>,
    pub schema: Arc<StateSchema>,
    pub checkpointer: Arc<dyn Checkpointer>,
}

impl StatefulRunner {
    pub async fn run(&self) -> Result<RunState> {
        // Resume from latest checkpoint if present.
        let mut state = match self.checkpointer.latest(&self.workflow_id, &self.run_id).await? {
            Some(snap) => RunState::from_snapshot(self.schema.clone(), snap.values, snap.key.super_step),
            None => RunState::new(self.schema.clone()),
        };
        let order = self.dag.topo_sort()?;
        // Group steps into super-steps by topological layer (level).
        let layers = self.layered(&order);
        let resume_at = state.super_step();
        let mut completed: HashSet<StepId> = HashSet::new();
        for (layer_idx, layer) in layers.iter().enumerate() {
            let super_step = layer_idx as u64 + 1;
            if super_step <= resume_at {
                for sid in layer {
                    completed.insert(sid.clone());
                }
                continue;
            }
            // Run the layer concurrently; collect all writes.
            let mut handles = Vec::new();
            for sid in layer {
                let step = self.dag.steps.get(sid).ok_or_else(|| {
                    AgentError::Workflow(format!("missing step {}", sid.as_str()))
                })?;
                let step = step.clone();
                let st = state.clone();
                handles.push(tokio::spawn(async move { step.run(&st).await }));
            }
            let mut all_writes: Vec<(String, Value)> = Vec::new();
            for h in handles {
                let writes = h.await.map_err(|e| AgentError::Internal(e.to_string()))??;
                all_writes.extend(writes);
            }
            state.merge_writes(all_writes)?;
            state.advance();
            for sid in layer {
                completed.insert(sid.clone());
            }
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
        }
        Ok(state)
    }

    fn layered(&self, order: &[StepId]) -> Vec<Vec<StepId>> {
        // Compute depth from edges; same depth = same super-step.
        use std::collections::HashMap;
        let mut depth: HashMap<StepId, usize> = HashMap::new();
        for s in order {
            depth.insert(s.clone(), 0);
        }
        for s in order {
            if let Some(succs) = self.dag.edges.get(s) {
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
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// Convenience: build a StatefulStep from a closure.
pub struct FnStatefulStep<F>(pub F);

#[async_trait]
impl<F, Fut> StatefulStep for FnStatefulStep<F>
where
    F: Fn(&RunState) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<Vec<(String, Value)>>> + Send + 'static,
{
    async fn run(&self, state: &RunState) -> Result<Vec<(String, Value)>> {
        (self.0)(state).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dag::Dag;
    use atomr_agents_state::{
        AppendMessages, InMemoryCheckpointer, MergeMap, StateSchema,
    };
    use serde_json::json;

    fn schema() -> Arc<StateSchema> {
        Arc::new(
            StateSchema::builder()
                .add("messages", AppendMessages)
                .add("config", MergeMap)
                .build(),
        )
    }

    fn step_writing<F>(write: F) -> Arc<dyn StatefulStep>
    where
        F: Fn(&RunState) -> Vec<(String, Value)> + Send + Sync + 'static,
    {
        Arc::new(FnStatefulStep(move |s: &RunState| {
            let writes = write(s);
            async move { Ok(writes) }
        }))
    }

    #[tokio::test]
    async fn linear_dag_writes_per_super_step() {
        let dag: Dag<Arc<dyn StatefulStep>> = Dag::builder("a")
            .step(
                "a",
                step_writing(|_| {
                    vec![("messages".into(), json!([{"id": "m1", "text": "hi"}]))]
                }),
            )
            .step(
                "b",
                step_writing(|s| {
                    let n = s.read("messages").as_array().map(|v| v.len()).unwrap_or(0);
                    vec![("config".into(), json!({"seen": n}))]
                }),
            )
            .edge("a", "b")
            .build();
        let runner = StatefulRunner {
            workflow_id: WorkflowId::from("wf"),
            run_id: RunId::from("r"),
            dag,
            schema: schema(),
            checkpointer: Arc::new(InMemoryCheckpointer::new()),
        };
        let final_state = runner.run().await.unwrap();
        assert_eq!(final_state.read("messages").as_array().unwrap().len(), 1);
        assert_eq!(final_state.read("config")["seen"], 1);
    }

    #[tokio::test]
    async fn resume_from_checkpoint_skips_completed_layers() {
        // First run completes layer 1; then we run again with the
        // same checkpointer and a step that would corrupt state if
        // re-executed. Resume must skip layer 1.
        let cpt = Arc::new(InMemoryCheckpointer::new());
        let bad: Arc<dyn StatefulStep> = Arc::new(FnStatefulStep(|_s: &RunState| async {
            Err::<Vec<(String, Value)>, _>(AgentError::Workflow("first run dies on b".into()))
        }));
        let dag1: Dag<Arc<dyn StatefulStep>> = Dag::builder("a")
            .step(
                "a",
                step_writing(|_| {
                    vec![("messages".into(), json!([{"id": "m1"}]))]
                }),
            )
            .step("b", bad)
            .edge("a", "b")
            .build();
        let r1 = StatefulRunner {
            workflow_id: WorkflowId::from("wf"),
            run_id: RunId::from("r"),
            dag: dag1,
            schema: schema(),
            checkpointer: cpt.clone(),
        };
        let _ = r1.run().await; // expected to fail
        // Layer 1 should be checkpointed.
        let metas = cpt
            .list(&WorkflowId::from("wf"), &RunId::from("r"))
            .await
            .unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].super_step, 1);

        // Second run: replace b with a benign step. a must NOT re-run
        // (it would dedupe via AppendMessages, so we'll detect by
        // counting how often a runs via a side-channel).
        use std::sync::atomic::{AtomicU32, Ordering};
        let a_runs = Arc::new(AtomicU32::new(0));
        let a_runs2 = a_runs.clone();
        let counted_a: Arc<dyn StatefulStep> = Arc::new(FnStatefulStep(move |_s: &RunState| {
            let c = a_runs2.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(vec![("messages".into(), json!([{"id": "m1"}]))])
            }
        }));
        let dag2: Dag<Arc<dyn StatefulStep>> = Dag::builder("a")
            .step("a", counted_a)
            .step(
                "b",
                step_writing(|_| vec![("config".into(), json!({"ok": true}))]),
            )
            .edge("a", "b")
            .build();
        let r2 = StatefulRunner {
            workflow_id: WorkflowId::from("wf"),
            run_id: RunId::from("r"),
            dag: dag2,
            schema: schema(),
            checkpointer: cpt.clone(),
        };
        let final_state = r2.run().await.unwrap();
        assert_eq!(a_runs.load(Ordering::SeqCst), 0);
        assert_eq!(final_state.read("config")["ok"], true);
    }
}
