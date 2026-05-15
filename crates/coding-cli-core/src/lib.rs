//! Uniform contract for the coding-cli harness.
//!
//! Adapters for individual CLIs (Claude Code, Codex, Gemini, ...) plug
//! into the harness via [`CliVendor`]. The harness consumes a
//! [`CliRequest`], spawns the CLI via an `Isolator` (sibling crate),
//! and produces a [`CliResult`] while broadcasting a stream of
//! normalized [`CodingCliEvent`]s.
//!
//! See the workspace `docs/coding-cli-harness.md` for the full design.

#![forbid(unsafe_code)]

mod error;
mod event;
mod projection;
mod request;
mod result;
mod vendor;

pub use error::{MapperError, ParseError};
pub use event::{
    CodingCliEvent, CodingCliEventStream, FinishReason, McpServerInit, ToolDescriptorInit,
};
pub use projection::{
    ConceptProjection, McpServerSnapshot, PersonaSnapshot, PolicySnapshot, SkillSnapshot,
    ToolSetSnapshot,
};
pub use request::{BudgetSpec, CliRequest, CliRunId, CliSessionId, IsolationSpec, RunMode};
pub use result::{CliResult, ToolCallRecord, UsageSummary};
pub use vendor::{CliCommand, CliEventParser, CliVendor, CliVendorKind};
