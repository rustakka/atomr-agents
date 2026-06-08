//! Warm-snapshot pool. Keeps a bounded set of resumable snapshots per profile
//! so a `fork` resolves to an instant resume instead of a cold boot. Used by
//! the single-host harness and (per-node) by the cluster control plane.

use std::collections::{HashMap, VecDeque};

use atomr_agents_sandbox_core::{SandboxProfile, SnapshotId};
use parking_lot::Mutex;

#[derive(Debug)]
pub struct SnapshotPool {
    inner: Mutex<HashMap<SandboxProfile, VecDeque<SnapshotId>>>,
    capacity_per_profile: usize,
}

impl SnapshotPool {
    pub fn new(capacity_per_profile: usize) -> Self {
        Self { inner: Mutex::new(HashMap::new()), capacity_per_profile }
    }

    /// Offer a warm snapshot to the pool. Returns `false` (and drops the
    /// snapshot) if the per-profile pool is already full — the caller should
    /// then discard the underlying memory file.
    pub fn offer(&self, profile: SandboxProfile, snap: SnapshotId) -> bool {
        let mut g = self.inner.lock();
        let q = g.entry(profile).or_default();
        if q.len() >= self.capacity_per_profile {
            return false;
        }
        q.push_back(snap);
        true
    }

    /// Take a warm snapshot for `profile`, if one is pooled.
    pub fn take(&self, profile: SandboxProfile) -> Option<SnapshotId> {
        self.inner.lock().get_mut(&profile).and_then(|q| q.pop_front())
    }

    /// Number of warm snapshots pooled for `profile`.
    pub fn len(&self, profile: SandboxProfile) -> usize {
        self.inner.lock().get(&profile).map_or(0, |q| q.len())
    }

    pub fn capacity_per_profile(&self) -> usize {
        self.capacity_per_profile
    }
}

impl Default for SnapshotPool {
    fn default() -> Self {
        Self::new(4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offer_take_respects_capacity() {
        let pool = SnapshotPool::new(2);
        assert!(pool.offer(SandboxProfile::PythonOnly, SnapshotId::new()));
        assert!(pool.offer(SandboxProfile::PythonOnly, SnapshotId::new()));
        // Third offer for the same profile is rejected.
        assert!(!pool.offer(SandboxProfile::PythonOnly, SnapshotId::new()));
        assert_eq!(pool.len(SandboxProfile::PythonOnly), 2);
        // A different profile has its own bucket.
        assert!(pool.offer(SandboxProfile::RustOnly, SnapshotId::new()));
        assert_eq!(pool.len(SandboxProfile::RustOnly), 1);

        assert!(pool.take(SandboxProfile::PythonOnly).is_some());
        assert_eq!(pool.len(SandboxProfile::PythonOnly), 1);
    }
}
