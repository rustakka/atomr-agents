//! Harness that wraps local AI coding CLIs as atomr-agents Callables.
//!
//! See `README.md` for the high-level architecture.

#![forbid(unsafe_code)]

mod dispatch;
mod error;
mod harness;
mod headless;
mod interactive;
mod pty_pump;
mod registry;
mod session;
mod spec;
mod store;

pub use atomr_agents_coding_cli_core::{
    CliCommand, CliEventParser, CliRequest, CliResult, CliRunId, CliSessionId, CliVendor,
    CliVendorKind, CodingCliEvent, CodingCliEventStream, ConceptProjection, FinishReason,
    IsolationSpec, McpServerInit, McpServerSnapshot, PersonaSnapshot, PolicySnapshot, RunMode,
    SkillSnapshot, ToolCallRecord, ToolDescriptorInit, ToolSetSnapshot, UsageSummary,
};

pub use error::HarnessError;
pub use harness::CodingCliHarness;
pub use registry::VendorRegistry;
pub use session::{InteractiveSessionHandle, SessionEvent, SessionTransport};
pub use spec::CodingCliHarnessSpec;
pub use store::{CliRunStore, InMemoryRunStore};
