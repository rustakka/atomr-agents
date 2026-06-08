//! Placement strategy for the clustered (Tier-3) topology. Lives in the
//! harness so the single-host path and the control plane share one
//! implementation.

use atomr_agents_sandbox_core::{ResourceBudget, SnapshotId};
use serde::{Deserialize, Serialize};

pub type NodeId = String;

/// A worker node's free capacity, reported via heartbeat. The scheduler
/// bin-packs sandboxes onto nodes using this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeStatus {
    pub id: NodeId,
    pub free_vcpus: u32,
    pub free_mem_mib: u32,
    /// Snapshots already resident on this node — used for fork locality.
    #[serde(default)]
    pub warm_snapshots: Vec<SnapshotId>,
}

/// Chooses a node for a sandbox of a given [`ResourceBudget`].
pub trait Scheduler: Send + Sync {
    /// Pick a node that fits `need`, optionally preferring one that already
    /// holds `prefer_snapshot` (fork locality). Returns `None` if nothing fits.
    fn place(
        &self,
        need: &ResourceBudget,
        prefer_snapshot: Option<&SnapshotId>,
        nodes: &[NodeStatus],
    ) -> Option<NodeId>;
}

/// Best-fit-decreasing bin packing: among nodes that fit, pick the one with
/// the *smallest* remaining capacity to maximize packing density. Ties break
/// toward a node already holding the preferred snapshot.
#[derive(Debug, Default, Clone, Copy)]
pub struct BestFitScheduler;

impl Scheduler for BestFitScheduler {
    fn place(
        &self,
        need: &ResourceBudget,
        prefer_snapshot: Option<&SnapshotId>,
        nodes: &[NodeStatus],
    ) -> Option<NodeId> {
        nodes
            .iter()
            .filter(|n| n.free_vcpus >= need.vcpus as u32 && n.free_mem_mib >= need.mem_mib)
            .min_by_key(|n| {
                // Sort key: snapshot-locality first (0 beats 1), then tightest
                // remaining capacity (best fit).
                let locality = match prefer_snapshot {
                    Some(s) if n.warm_snapshots.contains(s) => 0u8,
                    _ => 1u8,
                };
                (locality, n.free_mem_mib, n.free_vcpus)
            })
            .map(|n| n.id.clone())
    }
}

/// Test/dev scheduler: a fixed pick, else the first node.
#[derive(Debug, Default, Clone)]
pub struct MockScheduler {
    pub pick: Option<NodeId>,
}

impl Scheduler for MockScheduler {
    fn place(
        &self,
        _need: &ResourceBudget,
        _prefer_snapshot: Option<&SnapshotId>,
        nodes: &[NodeStatus],
    ) -> Option<NodeId> {
        self.pick
            .clone()
            .or_else(|| nodes.first().map(|n| n.id.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_sandbox_core::SandboxProfile;

    fn node(id: &str, vcpus: u32, mem: u32) -> NodeStatus {
        NodeStatus { id: id.into(), free_vcpus: vcpus, free_mem_mib: mem, warm_snapshots: vec![] }
    }

    #[test]
    fn best_fit_picks_tightest_fitting_node() {
        let need = ResourceBudget::for_profile(SandboxProfile::RustOnly); // 2 vcpu / 2048 mem
        let nodes = vec![
            node("big", 16, 32_000),
            node("snug", 2, 2_048),
            node("tiny", 1, 512), // does not fit
        ];
        let picked = BestFitScheduler.place(&need, None, &nodes);
        assert_eq!(picked.as_deref(), Some("snug"));
    }

    #[test]
    fn best_fit_returns_none_when_nothing_fits() {
        let need = ResourceBudget::for_profile(SandboxProfile::FullStack);
        let nodes = vec![node("tiny", 1, 256)];
        assert_eq!(BestFitScheduler.place(&need, None, &nodes), None);
    }

    #[test]
    fn snapshot_locality_breaks_ties() {
        let need = ResourceBudget::for_profile(SandboxProfile::PythonOnly); // 1 / 512
        let snap = SnapshotId::new();
        let mut local = node("local", 4, 4_096);
        local.warm_snapshots.push(snap.clone());
        let remote = node("remote", 4, 4_096);
        // Same capacity; the node holding the snapshot wins.
        let picked = BestFitScheduler.place(&need, Some(&snap), &[remote, local]);
        assert_eq!(picked.as_deref(), Some("local"));
    }
}
