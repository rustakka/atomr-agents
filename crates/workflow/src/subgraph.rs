//! Subgraphs with shared channels.
//!
//! A `Subgraph` is a `StatefulRunner` packaged so a parent workflow
//! can call it as a step. The parent declares two projection lists:
//!
//! - `input_channels`: keys read from the parent state and passed
//!   into the child as initial values.
//! - `output_channels`: keys read from the child's final state and
//!   merged back into the parent through the parent's reducers.
//!
//! Channels not in either list are *private* to the child.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::Callable;
use atomr_agents_core::{AgentError, CallCtx, Result, RunId, Value, WorkflowId};
use atomr_agents_state::{Checkpointer, RunState, StateSchema};

use crate::dag::Dag;
use crate::state_runner::{StatefulRunner, StatefulStep};

/// Subgraph-as-callable. Returns a JSON object with two keys:
/// `outputs` (the projected output channels) and `private_state`
/// (the full child snapshot, included when callers want to inspect
/// child-only channels).
pub struct Subgraph {
    pub workflow_id: WorkflowId,
    pub run_id: RunId,
    pub dag: Dag<Arc<dyn StatefulStep>>,
    pub schema: Arc<StateSchema>,
    pub checkpointer: Arc<dyn Checkpointer>,
    pub input_channels: Vec<String>,
    pub output_channels: Vec<String>,
}

#[async_trait]
impl Callable for Subgraph {
    async fn call(&self, input: Value, _ctx: CallCtx) -> Result<Value> {
        // Build the child's RunState by projecting the parent input
        // (a JSON object) through `input_channels`.
        let parent_obj = match input {
            Value::Object(m) => m,
            other => {
                return Err(AgentError::Workflow(format!(
                    "subgraph: expected object input, got {other}"
                )));
            }
        };
        let mut child_state = RunState::new(self.schema.clone());
        let mut writes = Vec::new();
        for k in &self.input_channels {
            if let Some(v) = parent_obj.get(k) {
                writes.push((k.clone(), v.clone()));
            }
        }
        child_state.merge_writes(writes)?;

        // Persist the seeded state as super_step 0 so the runner
        // resumes from there (i.e. it'll skip super_step 0 and start
        // running the first DAG layer).
        self.checkpointer
            .save(atomr_agents_state::Snapshot {
                key: atomr_agents_state::CheckpointKey {
                    workflow_id: self.workflow_id.clone(),
                    run_id: self.run_id.clone(),
                    super_step: 0,
                },
                values: child_state.snapshot(),
                label: "subgraph-seed".into(),
                timestamp_ms: now_ms(),
            })
            .await?;

        let runner = StatefulRunner {
            workflow_id: self.workflow_id.clone(),
            run_id: self.run_id.clone(),
            dag: clone_dag(&self.dag),
            schema: self.schema.clone(),
            checkpointer: self.checkpointer.clone(),
        };
        let final_state = runner.run().await?;
        let mut outputs = serde_json::Map::new();
        for k in &self.output_channels {
            outputs.insert(k.clone(), final_state.read(k).clone());
        }
        Ok(serde_json::json!({
            "outputs": Value::Object(outputs),
            "private_state": final_state.snapshot(),
        }))
    }

    fn label(&self) -> &str {
        self.workflow_id.as_str()
    }
}

fn clone_dag(d: &Dag<Arc<dyn StatefulStep>>) -> Dag<Arc<dyn StatefulStep>> {
    Dag {
        steps: d.steps.clone(),
        edges: d.edges.clone(),
        entry: d.entry.clone(),
    }
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
    use crate::dag::Dag;
    use crate::state_runner::FnStatefulStep;
    use atomr_agents_core::{IterationBudget, MoneyBudget, TimeBudget, TokenBudget};
    use atomr_agents_state::{
        AppendMessages, InMemoryCheckpointer, MergeMap, StateSchema,
    };
    use serde_json::json;
    use std::time::Duration;

    fn child_schema() -> Arc<StateSchema> {
        Arc::new(
            StateSchema::builder()
                .add("messages", AppendMessages)
                .add("notes", MergeMap)
                .build(),
        )
    }

    fn ctx() -> CallCtx {
        CallCtx {
            agent_id: None,
            tokens: TokenBudget::new(1000),
            time: TimeBudget::new(Duration::from_secs(5)),
            money: MoneyBudget::from_usd(0.10),
            iterations: IterationBudget::new(10),
            trace: vec![],
        }
    }

    fn child_step() -> Arc<dyn StatefulStep> {
        Arc::new(FnStatefulStep(|s: &RunState| {
            let n = s.read("messages").as_array().map(|v| v.len()).unwrap_or(0);
            async move {
                Ok(vec![
                    (
                        "messages".into(),
                        json!([{"id": format!("c-{n}"), "text": "child added"}]),
                    ),
                    ("notes".into(), json!({"child_saw": n})),
                ])
            }
        }))
    }

    #[tokio::test]
    async fn subgraph_projects_in_then_out() {
        let dag: Dag<Arc<dyn StatefulStep>> = Dag::builder("a").step("a", child_step()).build();
        let sub = Subgraph {
            workflow_id: WorkflowId::from("child-wf"),
            run_id: RunId::from("child-run"),
            dag,
            schema: child_schema(),
            checkpointer: Arc::new(InMemoryCheckpointer::new()),
            input_channels: vec!["messages".into()],
            output_channels: vec!["notes".into()],
        };
        let parent_input = json!({
            "messages": [{"id": "p-1", "text": "from parent"}],
            "config": {"unrelated": true},
        });
        let out = sub.call(parent_input, ctx()).await.unwrap();
        // Output projected.
        assert!(out["outputs"]["notes"]["child_saw"].is_number());
        // private_state contains messages too (full child snapshot).
        assert!(out["private_state"]["messages"].is_array());
    }
}
