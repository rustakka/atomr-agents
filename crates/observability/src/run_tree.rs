//! `RunTree` builder.
//!
//! Aggregates `EventEnvelope`s into a parent-child tree keyed by
//! `run_id`. Used by tracer exporters and the Studio inspector.

use std::collections::HashMap;

use atomr_agents_core::{Event, EventEnvelope, RunId, Value};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunKind {
    Chain,
    Llm,
    Tool,
    Retriever,
    Parser,
    Agent,
    Workflow,
    Harness,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunNode {
    pub run_id: RunId,
    pub parent_run_id: Option<RunId>,
    pub kind: RunKind,
    pub name: String,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    pub tags: Vec<String>,
    pub events: Vec<EventEnvelope>,
    pub children: Vec<RunId>,
    pub error: Option<String>,
}

impl RunNode {
    pub fn elapsed_ms(&self) -> Option<i64> {
        self.ended_at_ms.map(|e| e - self.started_at_ms)
    }
}

/// Buffers events keyed by `run_id`. Subscribers attach the builder
/// to an `EventBus`; `snapshot` returns the current tree.
#[derive(Default)]
pub struct RunTreeBuilder {
    nodes: RwLock<HashMap<RunId, RunNode>>,
    /// Insertion order so the tree returned has a deterministic root list.
    order: RwLock<Vec<RunId>>,
}

impl RunTreeBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ingest(&self, env: &EventEnvelope) {
        let Some(rid) = env.run_id.clone() else { return };
        let mut nodes = self.nodes.write();
        let node = nodes.entry(rid.clone()).or_insert_with(|| {
            self.order.write().push(rid.clone());
            RunNode {
                run_id: rid.clone(),
                parent_run_id: env.parent_run_id.clone(),
                kind: classify(&env.event),
                name: name_of(&env.event),
                started_at_ms: env.timestamp_ms,
                ended_at_ms: None,
                tags: env.tags.clone(),
                events: Vec::new(),
                children: Vec::new(),
                error: None,
            }
        });
        if env.parent_run_id.is_some() && node.parent_run_id.is_none() {
            node.parent_run_id = env.parent_run_id.clone();
        }
        node.events.push(env.clone());
        // Heuristic: if event reports completion, set ended_at.
        if let Some(ms) = end_timestamp_for(env) {
            node.ended_at_ms = Some(ms);
        }
        if !env.tags.is_empty() {
            for t in &env.tags {
                if !node.tags.contains(t) {
                    node.tags.push(t.clone());
                }
            }
        }
        // Wire the child into its parent's children list.
        if let Some(parent) = env.parent_run_id.clone() {
            let child_id = rid.clone();
            // Drop the borrow on `node` before re-borrowing `nodes`.
            let _ = node;
            let parent_node = nodes.entry(parent.clone()).or_insert_with(|| {
                self.order.write().push(parent.clone());
                RunNode {
                    run_id: parent.clone(),
                    parent_run_id: None,
                    kind: RunKind::Other,
                    name: "(parent)".into(),
                    started_at_ms: env.timestamp_ms,
                    ended_at_ms: None,
                    tags: vec![],
                    events: Vec::new(),
                    children: Vec::new(),
                    error: None,
                }
            });
            if !parent_node.children.contains(&child_id) {
                parent_node.children.push(child_id);
            }
        }
    }

    /// Returns all root nodes (those without a parent) in insertion order.
    pub fn roots(&self) -> Vec<RunNode> {
        let nodes = self.nodes.read();
        let order = self.order.read();
        order
            .iter()
            .filter_map(|id| nodes.get(id).cloned())
            .filter(|n| n.parent_run_id.is_none())
            .collect()
    }

    /// Returns a snapshot of every node by id.
    pub fn snapshot(&self) -> HashMap<RunId, RunNode> {
        self.nodes.read().clone()
    }

    pub fn get(&self, id: &RunId) -> Option<RunNode> {
        self.nodes.read().get(id).cloned()
    }

    /// Subscribe this builder to an `EventBus`.
    pub fn attach(self: std::sync::Arc<Self>, bus: &crate::EventBus) {
        let me = self;
        bus.subscribe(move |env| me.ingest(env));
    }
}

fn classify(e: &Event) -> RunKind {
    match e {
        Event::AgentTurn { .. } => RunKind::Agent,
        Event::ToolInvoked { .. } => RunKind::Tool,
        Event::WorkflowStep { .. } => RunKind::Workflow,
        Event::HarnessIteration { .. } => RunKind::Harness,
        Event::StrategyResolved { .. } => RunKind::Chain,
        Event::Backpressure { .. } => RunKind::Other,
    }
}

fn name_of(e: &Event) -> String {
    match e {
        Event::AgentTurn { agent_id, .. } => format!("agent:{}", agent_id.as_str()),
        Event::ToolInvoked { tool_id, .. } => format!("tool:{}", tool_id.as_str()),
        Event::WorkflowStep { step_id, .. } => format!("step:{step_id}"),
        Event::HarnessIteration {
            harness_id,
            iteration,
            ..
        } => {
            format!("harness:{}#{iteration}", harness_id.as_str())
        }
        Event::StrategyResolved { strategy, .. } => format!("strategy:{strategy}"),
        Event::Backpressure { actor_path, .. } => format!("backpressure:{actor_path}"),
    }
}

fn end_timestamp_for(env: &EventEnvelope) -> Option<i64> {
    match &env.event {
        Event::AgentTurn { .. }
        | Event::ToolInvoked { .. }
        | Event::WorkflowStep { .. }
        | Event::HarnessIteration { .. }
        | Event::StrategyResolved { .. } => Some(env.timestamp_ms),
        _ => None,
    }
}

#[allow(dead_code)]
fn _value_in_scope(_v: Value) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EventBus;
    use atomr_agents_core::{AgentId, ToolId};
    use std::sync::Arc;

    #[test]
    fn run_tree_links_parent_and_child() {
        let bus = EventBus::new();
        let builder = Arc::new(RunTreeBuilder::new());
        builder.clone().attach(&bus);

        let parent = RunId::from("r-parent");
        let child = RunId::from("r-child");

        bus.emit_run(
            Event::AgentTurn {
                agent_id: AgentId::from("a"),
                input_tokens: 5,
                output_tokens: 5,
                finish_reason: None,
                elapsed_ms: 10,
            },
            parent.clone(),
            None,
        );
        bus.emit_run(
            Event::ToolInvoked {
                tool_id: ToolId::from("t"),
                args_hash: 1,
                elapsed_ms: 2,
                ok: true,
            },
            child.clone(),
            Some(parent.clone()),
        );

        let p = builder.get(&parent).unwrap();
        assert_eq!(p.kind, RunKind::Agent);
        assert_eq!(p.children.len(), 1);
        assert_eq!(p.children[0].as_str(), child.as_str());
        let c = builder.get(&child).unwrap();
        assert_eq!(c.kind, RunKind::Tool);
        assert_eq!(c.parent_run_id.unwrap().as_str(), parent.as_str());
    }
}
