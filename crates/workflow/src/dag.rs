use std::collections::{BTreeMap, HashMap};

use atomr_agents_core::{AgentError, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StepId(pub String);

impl StepId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for StepId {
    fn from(s: &str) -> Self {
        Self(s.into())
    }
}

/// Static structure of a workflow. Steps are stored separately from
/// their adjacency to keep the runtime types simple.
pub struct Dag<S> {
    pub steps: BTreeMap<StepId, S>,
    pub edges: HashMap<StepId, Vec<StepId>>,
    pub entry: StepId,
}

impl<S> Dag<S> {
    pub fn builder(entry: impl Into<StepId>) -> DagBuilder<S> {
        DagBuilder { steps: BTreeMap::new(), edges: HashMap::new(), entry: entry.into() }
    }

    /// Topological order of step ids. Errors on cycles.
    pub fn topo_sort(&self) -> Result<Vec<StepId>> {
        let mut indeg: HashMap<StepId, usize> = self.steps.keys().map(|k| (k.clone(), 0)).collect();
        for tos in self.edges.values() {
            for to in tos {
                if let Some(d) = indeg.get_mut(to) {
                    *d += 1;
                }
            }
        }
        let mut queue: Vec<StepId> = indeg
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(k, _)| k.clone())
            .collect();
        queue.sort();
        let mut out = Vec::with_capacity(self.steps.len());
        while let Some(n) = queue.pop() {
            out.push(n.clone());
            if let Some(succ) = self.edges.get(&n) {
                for s in succ {
                    if let Some(d) = indeg.get_mut(s) {
                        *d -= 1;
                        if *d == 0 {
                            queue.push(s.clone());
                        }
                    }
                }
            }
            queue.sort();
        }
        if out.len() != self.steps.len() {
            return Err(AgentError::Workflow("dag has a cycle".into()));
        }
        Ok(out)
    }
}

pub struct DagBuilder<S> {
    steps: BTreeMap<StepId, S>,
    edges: HashMap<StepId, Vec<StepId>>,
    entry: StepId,
}

impl<S> DagBuilder<S> {
    pub fn step(mut self, id: impl Into<StepId>, step: S) -> Self {
        self.steps.insert(id.into(), step);
        self
    }

    pub fn edge(mut self, from: impl Into<StepId>, to: impl Into<StepId>) -> Self {
        self.edges.entry(from.into()).or_default().push(to.into());
        self
    }

    pub fn build(self) -> Dag<S> {
        Dag { steps: self.steps, edges: self.edges, entry: self.entry }
    }
}
