//! Harness wrapping Anthropic's **Claude Agent SDK** as an atomr-agents
//! [`Callable`](atomr_agents_callable::Callable).
//!
//! The harness composes an [`AgentSdkBackend`](atomr_agents_agent_sdk_core::AgentSdkBackend)
//! (mock for tests, the Python-driven SDK in production) with an event
//! broadcast, a session registry, an Anthropic-credit spend ledger, and a
//! `.claude/` projection. It runs one-shot queries (headless) and stateful
//! interactive sessions, and — behind the `actor` feature — exposes the
//! session as an `atomr_core::actor::Actor`.
//!
//! # Safety
//!
//! The default [`PermissionMode`](atomr_agents_agent_sdk_core::PermissionMode)
//! is `bypassPermissions` — the agent auto-approves every tool, including
//! `Bash`, `Write`, and network access. This is the configured default for
//! autonomy; it is overridable per request/spec. The harness validates
//! `cwd`/`add_dirs` as real directories and defaults `setting_sources` to
//! `["project"]` so the agent only sees atomr-materialized config. Run
//! untrusted work inside the sandbox harness.

#![forbid(unsafe_code)]

mod budget;
mod bridge;
mod error;
mod harness;
mod headless;
mod projection;
mod session;
mod spec;

#[cfg(feature = "actor")]
pub mod actor;

#[cfg(feature = "sandbox")]
pub mod workspace;

pub use error::{HarnessError, Result};
pub use harness::AgentSdkHarness;
pub use projection::{render_projection, CommandDoc, Projection, SkillDoc};
pub use session::{InteractiveAgentSession, SessionRegistry};
pub use spec::{AgentSdkHarnessSpec, AuthConfig, AuthProvider};

#[cfg(feature = "sandbox")]
pub use workspace::{
    SandboxWorkspaceConfig, SessionWorkspace, WorkspaceDisposition, WorkspaceRegistry,
};

// Re-export the contract types so downstream crates have one import path.
pub use atomr_agents_agent_sdk_core as core;
