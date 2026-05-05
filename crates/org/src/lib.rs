//! Organizational hierarchy: Org → Department → Team → Unit.

mod memory;
mod patterns;
mod routing;
mod team;

pub use memory::{NamespacedMemory, OrgMemoryView};
pub use patterns::{swarm_loop, ActiveAgent};
pub use routing::{CapabilityMatchRouter, LoadAwareRouter, OrgRoutingStrategy, RoundRobinRouter};
pub use team::{Department, Org, Team};
