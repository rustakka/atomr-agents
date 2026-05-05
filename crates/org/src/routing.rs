use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::CallableHandle;
use atomr_agents_core::{AgentError, Result};

/// Routing strategy used by Org/Department/Team. Differs from
/// `atomr_agents_strategy::RoutingStrategy` by being keyed on the
/// child's *label* rather than `AgentContext` — these run pre-agent.
#[async_trait]
pub trait OrgRoutingStrategy: Send + Sync + 'static {
    async fn pick(
        &self,
        children: &[(String, CallableHandle)],
        request_label: &str,
    ) -> Result<CallableHandle>;
}

/// Round-robin among the children. Stateful via `AtomicUsize`.
pub struct RoundRobinRouter {
    cursor: Arc<AtomicUsize>,
}

impl RoundRobinRouter {
    pub fn new() -> Self {
        Self { cursor: Arc::new(AtomicUsize::new(0)) }
    }
}

impl Default for RoundRobinRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl OrgRoutingStrategy for RoundRobinRouter {
    async fn pick(
        &self,
        children: &[(String, CallableHandle)],
        _label: &str,
    ) -> Result<CallableHandle> {
        if children.is_empty() {
            return Err(AgentError::Internal("round-robin: no children".into()));
        }
        let i = self.cursor.fetch_add(1, Ordering::SeqCst) % children.len();
        Ok(children[i].1.clone())
    }
}

/// Load-aware: tracks an integer "in-flight" counter per child and
/// picks the lightest. v0 is best-effort (counter is incremented at
/// pick time, never decremented — so it approximates a least-recently-
/// picked policy). A real implementation pairs with `tell_done` after
/// the child completes; we'll add that hook when actor-spawning lands.
pub struct LoadAwareRouter {
    inflight: Arc<parking_lot::Mutex<Vec<u32>>>,
}

impl LoadAwareRouter {
    pub fn for_size(n: usize) -> Self {
        Self { inflight: Arc::new(parking_lot::Mutex::new(vec![0; n])) }
    }
}

#[async_trait]
impl OrgRoutingStrategy for LoadAwareRouter {
    async fn pick(
        &self,
        children: &[(String, CallableHandle)],
        _label: &str,
    ) -> Result<CallableHandle> {
        if children.is_empty() {
            return Err(AgentError::Internal("load-aware: no children".into()));
        }
        let mut g = self.inflight.lock();
        if g.len() != children.len() {
            g.resize(children.len(), 0);
        }
        let (idx, _) = g
            .iter()
            .enumerate()
            .min_by_key(|(_, c)| **c)
            .unwrap();
        g[idx] += 1;
        Ok(children[idx].1.clone())
    }
}

/// Capability match: each child carries a label like "L1" or "L2"; the
/// router picks the child whose label *contains* `request_label`. Falls
/// back to the first child if no match.
pub struct CapabilityMatchRouter;

#[async_trait]
impl OrgRoutingStrategy for CapabilityMatchRouter {
    async fn pick(
        &self,
        children: &[(String, CallableHandle)],
        request_label: &str,
    ) -> Result<CallableHandle> {
        if children.is_empty() {
            return Err(AgentError::Internal("capability-match: no children".into()));
        }
        let needle = request_label.to_lowercase();
        let chosen = children
            .iter()
            .find(|(label, _)| label.to_lowercase().contains(&needle))
            .map(|(_, h)| h.clone())
            .unwrap_or_else(|| children[0].1.clone());
        Ok(chosen)
    }
}
