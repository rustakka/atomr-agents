//! Organizational hierarchy: Org → Department → Team → Unit.
//!
//! Beyond the routing hierarchy this crate also hosts two governance
//! primitives:
//!
//! * [`projection`] (FR-13) — projects an `atomr-orgs` system-of-record
//!   org onto this routing org, keeping the compartment information-wall
//!   in sync (fail-closed) and providing a CI drift gate.
//! * [`debate`] (FR-17) — a reusable adversarial debate/critique
//!   protocol for trustworthy thesis verification.

mod debate;
mod memory;
mod patterns;
mod projection;
mod routing;
mod team;

pub use debate::{
    CritiqueChannel, DebateOutcome, DebateRoles, DebateState, DebateStrategy, DebateTurn, Dissent,
    DissentTermination, Stance, CRITIQUE_CHANNEL,
};
pub use memory::{NamespacedMemory, OrgMemoryView};
pub use patterns::{swarm_loop, ActiveAgent};
pub use projection::{
    CompartmentMapping, CompiledOrgModel, CompiledOrgSource, CompiledRole, CompiledUnit, Drift, DriftKind,
    OrgProjection, ProjectedOrg, ProjectedUnit, ProjectionError, WallSyncReport,
};
pub use routing::{CapabilityMatchRouter, LoadAwareRouter, OrgRoutingStrategy, RoundRobinRouter};
pub use team::{Department, Org, Team};
