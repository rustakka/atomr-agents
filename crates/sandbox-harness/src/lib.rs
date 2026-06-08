//! `atomr-agents-sandbox-harness` — orchestration for the microVM sandbox.
//!
//! The harness owns a pluggable [`SandboxBackend`](atomr_agents_sandbox_core::SandboxBackend),
//! a registry of live sandboxes, a bin-packing [`Scheduler`] and a warm
//! [`SnapshotPool`], an `EventBus`, and a broadcast channel of
//! [`SandboxEvent`](atomr_agents_sandbox_core::SandboxEvent)s. It exposes both
//! an ephemeral one-shot path ([`SandboxHarness::run_once`], used by the
//! `execute_in_sandbox` tool) and a persistent registry path
//! ([`SandboxHarness::create`] / [`exec`](SandboxHarness::exec) /
//! [`fork`](SandboxHarness::fork) / [`destroy`](SandboxHarness::destroy), used
//! by the PyO3 `SandboxClient`). It is itself a `Callable`.
//!
//! Mirrors `coding-cli-harness`.

#![forbid(unsafe_code)]

mod harness;
mod pool;
mod scheduler;

pub use harness::{SandboxHarness, SandboxHarnessConfig};
pub use pool::SnapshotPool;
pub use scheduler::{BestFitScheduler, MockScheduler, NodeId, NodeStatus, Scheduler};
