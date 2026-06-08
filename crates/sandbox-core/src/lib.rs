//! `atomr-agents-sandbox-core` — backend-agnostic types and traits for the
//! agent microVM sandbox subsystem.
//!
//! This crate is the contract every sandbox backend implements and every
//! consumer (the `execute_in_sandbox` tool, the harness, the PyO3 bindings,
//! the gRPC cluster) depends on. It carries no hypervisor or container code —
//! only domain types, the [`SandboxBackend`] / [`SandboxHandle`] traits, the
//! [`SandboxEvent`] stream, and a deterministic [`MockBackend`] so the whole
//! surface is testable without Docker or KVM.
//!
//! The decomposition mirrors `web-search-core` (trait + types + in-crate
//! mock) and `coding-cli-isolator` (the `Isolator` / `ProcessHandle` shapes
//! that [`SandboxBackend`] / [`SandboxHandle`] are modeled on).

#![forbid(unsafe_code)]

mod backend;
mod budget;
mod error;
mod event;
mod exit;
mod id;
mod mock;
mod profile;
mod request;

pub use backend::{ExecStream, SandboxBackend, SandboxHandle};
pub use budget::ResourceBudget;
pub use error::{Result, SandboxError};
pub use event::{SandboxEvent, SandboxEventStream};
pub use exit::ExitStatus;
pub use id::{ExecId, SandboxId, SnapshotId};
pub use mock::{MockBackend, MockHandle};
pub use profile::{Language, SandboxProfile};
pub use request::{CreateSandbox, ExecRequest, ExecResult, SandboxBackendSel, SandboxInfo};
