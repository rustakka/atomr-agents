//! # atomr-agents-host
//!
//! Long-lived agent identity built on top of [`atomr`] actors.
//!
//! The host gives an atomr-agents agent persistent identity, memory,
//! skills, rules, tools, hooks, schedules, and inbound channels — the
//! same role Claude Code plays for the Claude model.
//!
//! ## Surface
//!
//! - [`HostConfig`] + [`HostPaths`] / [`AgentPaths`] — on-disk layout.
//! - [`AgentLoader`] / [`AgentDefinition`] / [`LoadedAgent`] — read
//!   `agents/<id>/` and assemble `AgentSpec` + `SkillSet` + `PersonaValue`.
//! - [`runtime::HostRuntime`] — owns the `ActorSystem`, spawns
//!   [`actor::AgentHostActor`] per loaded agent, exposes typed handles.
//!
//! Subsequent modules (`chat`, `memory_sync`, `skills_registry`,
//! `hooks`, `scheduler`, `gateway`, `mcp`, `curator`, `branching`,
//! `registry_cache`, `evals`) layer the M2-M12 milestones on top.

#![allow(clippy::result_large_err)]

pub mod actor;
pub mod branching;
pub mod chat;
pub mod config;
pub mod curator;
pub mod error;
pub mod evals;
pub mod events;
pub mod gateway;
pub mod hooks;
pub mod layout;
pub mod loader;
pub mod markdown;
pub mod mcp;
pub mod memory_sync;
pub mod registry_cache;
pub mod routes;
pub mod runtime;
pub mod scheduler;
pub mod skills_registry;

pub use config::{HostConfig, ProviderConfig};
pub use error::HostError;
pub use layout::{default_root, AgentPaths, HostPaths};
pub use loader::{
    AgentDefinition, AgentLoader, HookDefinition, LoadedAgent, SkillDefinition,
};
pub use markdown::MarkdownDoc;
pub use runtime::HostRuntime;
