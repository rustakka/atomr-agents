//! Studio-style read+resume inspector.
//!
//! Provides typed handler functions over `Checkpointer` + journal +
//! resume API. The CLI's `serve` subcommand wires these into an HTTP
//! router; tests exercise them directly so we don't need a live
//! socket.

use std::sync::Arc;

use atomr_agents_core::{Result, RunId, Value, WorkflowId};
use atomr_agents_state::{CheckpointKey, CheckpointMeta, Checkpointer, Snapshot};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummary {
    pub workflow_id: WorkflowId,
    pub run_id: RunId,
    pub super_steps: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateView {
    pub snapshot: Snapshot,
}

pub struct Inspector {
    pub checkpointer: Arc<dyn Checkpointer>,
}

impl Inspector {
    pub fn new(checkpointer: Arc<dyn Checkpointer>) -> Self {
        Self { checkpointer }
    }

    /// `GET /runs/:workflow/:run/checkpoints` — list super-step ids.
    pub async fn list_checkpoints(
        &self,
        workflow_id: &WorkflowId,
        run_id: &RunId,
    ) -> Result<Vec<CheckpointMeta>> {
        self.checkpointer.list(workflow_id, run_id).await
    }

    /// `GET /runs/:workflow/:run/checkpoints/:step` — full state.
    pub async fn get_checkpoint(
        &self,
        workflow_id: &WorkflowId,
        run_id: &RunId,
        super_step: u64,
    ) -> Result<Option<StateView>> {
        let key = CheckpointKey {
            workflow_id: workflow_id.clone(),
            run_id: run_id.clone(),
            super_step,
        };
        let snap = self.checkpointer.load(&key).await?;
        Ok(snap.map(|s| StateView { snapshot: s }))
    }

    /// `POST /runs/:workflow/:run/fork` — divergent run with edits.
    pub async fn fork(
        &self,
        workflow_id: &WorkflowId,
        run_id: &RunId,
        super_step: u64,
        edits: Vec<(String, Value)>,
    ) -> Result<RunId> {
        let key = CheckpointKey {
            workflow_id: workflow_id.clone(),
            run_id: run_id.clone(),
            super_step,
        };
        self.checkpointer.fork(&key, edits).await
    }
}

/// Render a minimal HTML page summarizing a run. Used by the
/// `GET /runs/:workflow/:run` endpoint to produce a screenshot-credible
/// view without a JS framework.
pub fn render_run_html(summary: &RunSummary, checkpoints: &[CheckpointMeta]) -> String {
    let mut rows = String::new();
    for c in checkpoints {
        rows.push_str(&format!(
            "<tr><td>{}</td><td>{}</td></tr>",
            c.super_step, c.timestamp_ms
        ));
    }
    format!(
        r#"<!doctype html>
<html><head><title>Run {run}</title></head>
<body>
<h1>{wf} — {run}</h1>
<p>Super-steps: {n}</p>
<table border=1 cellpadding=4>
<tr><th>Super-step</th><th>Timestamp ms</th></tr>
{rows}
</table>
</body></html>"#,
        wf = summary.workflow_id.as_str(),
        run = summary.run_id.as_str(),
        n = summary.super_steps.len(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_state::InMemoryCheckpointer;
    use std::collections::HashMap;

    fn snap(wf: &str, run: &str, step: u64, kvs: Vec<(&str, Value)>) -> Snapshot {
        let mut values = HashMap::new();
        for (k, v) in kvs {
            values.insert(k.into(), v);
        }
        Snapshot {
            key: CheckpointKey {
                workflow_id: WorkflowId::from(wf),
                run_id: RunId::from(run),
                super_step: step,
            },
            values,
            label: format!("step-{step}"),
            timestamp_ms: 1_000 * step as i64,
        }
    }

    async fn setup() -> Inspector {
        let cpt = Arc::new(InMemoryCheckpointer::new());
        for step in [0, 1, 2, 3] {
            cpt.save(snap("wf", "r", step, vec![("messages", serde_json::json!([])), ("step", serde_json::json!(step))]))
                .await
                .unwrap();
        }
        Inspector::new(cpt)
    }

    #[tokio::test]
    async fn list_returns_all_super_steps() {
        let i = setup().await;
        let metas = i
            .list_checkpoints(&WorkflowId::from("wf"), &RunId::from("r"))
            .await
            .unwrap();
        assert_eq!(metas.len(), 4);
    }

    #[tokio::test]
    async fn get_specific_checkpoint_returns_state() {
        let i = setup().await;
        let v = i
            .get_checkpoint(&WorkflowId::from("wf"), &RunId::from("r"), 2)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(v.snapshot.values["step"], 2);
    }

    #[tokio::test]
    async fn fork_creates_divergent_run() {
        let i = setup().await;
        let new_run = i
            .fork(
                &WorkflowId::from("wf"),
                &RunId::from("r"),
                2,
                vec![("step".into(), serde_json::json!(99))],
            )
            .await
            .unwrap();
        // The forked run has step=99 at super_step 2.
        let v = i
            .get_checkpoint(&WorkflowId::from("wf"), &new_run, 2)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(v.snapshot.values["step"], 99);
    }

    #[test]
    fn render_html_includes_summary_rows() {
        let summary = RunSummary {
            workflow_id: WorkflowId::from("wf"),
            run_id: RunId::from("r"),
            super_steps: vec![0, 1, 2],
        };
        let cps = vec![
            CheckpointMeta {
                workflow_id: WorkflowId::from("wf"),
                run_id: RunId::from("r"),
                super_step: 0,
                timestamp_ms: 0,
            },
            CheckpointMeta {
                workflow_id: WorkflowId::from("wf"),
                run_id: RunId::from("r"),
                super_step: 1,
                timestamp_ms: 1000,
            },
        ];
        let html = render_run_html(&summary, &cps);
        assert!(html.contains("wf — r"));
        assert!(html.contains("Super-steps: 3"));
        assert!(html.contains("<td>0</td>"));
    }
}
