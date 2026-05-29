//! Typed security context for atomr-agents.
//!
//! This crate carries the caller's identity and authority through the
//! call/tool context — at the substrate, not in the prompt — so that:
//!
//! * agents **never hold credentials** (they hold opaque
//!   [`CapabilityHandle`]s; a [`CapabilityBroker`] resolves them to a
//!   use-and-drop [`ScopedSecret`] only after a clearance check);
//! * every guarded tool is checked against an information wall and a
//!   [`Mandate`] boundary via reusable [`WalledTool`] middleware;
//! * clearance flows as a typed [`atomr_agents_core::CallCtx`] extension
//!   ([`ClearanceContext`]) that is never serialized into a prompt,
//!   checkpoint, or telemetry record and is not LLM-writable.
//!
//! These are framework-local abstractions. The system-of-record
//! (`atomr-orgs`) projects its clearance/compartment model onto
//! [`ClearanceContext`] / [`Compartment`].

mod clearance;
mod error;
mod mandate;
mod secret;
mod walled;

pub use clearance::{ClearanceContext, ClearanceLevel, Compartment, NeedToKnow};
pub use error::SecurityError;
pub use mandate::{AllowAll, DenyAll, Mandate};
pub use secret::{CapabilityBroker, CapabilityHandle, ScopedSecret, StaticCapabilityBroker};
pub use walled::WalledTool;
