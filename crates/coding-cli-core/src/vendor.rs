//! The `CliVendor` integration seam.
//!
//! Each supported CLI (Claude Code, Codex, Gemini, ...) lives in its
//! own crate (`atomr-agents-coding-cli-vendor-<name>`) that implements
//! this trait. The harness composes vendor adapters into a registry
//! and dispatches based on `CliRequest::vendor`.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::{MapperError, ParseError};
use crate::event::CodingCliEvent;
use crate::projection::ConceptProjection;
use crate::request::CliRequest;

/// Stable identifier for a vendor adapter. Extensible — third-party
/// adapters can use [`CliVendorKind::Other`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliVendorKind {
    Claude,
    Codex,
    Gemini,
    Cursor,
    Aider,
    Other(String),
}

impl CliVendorKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Cursor => "cursor",
            Self::Aider => "aider",
            Self::Other(s) => s.as_str(),
        }
    }
}

impl std::fmt::Display for CliVendorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A concrete process invocation produced by a vendor adapter.
///
/// Isolators consume this to spawn either a host `tokio::process` or
/// an in-container exec — the spec is identical in both worlds.
#[derive(Debug, Clone)]
pub struct CliCommand {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: BTreeMap<String, String>,
    /// Working directory inside whatever environment runs the command.
    pub workdir: PathBuf,
    /// If `true`, the isolator must allocate a PTY (interactive mode).
    pub allocate_pty: bool,
}

impl CliCommand {
    pub fn new(program: impl Into<PathBuf>, workdir: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env: BTreeMap::new(),
            workdir: workdir.into(),
            allocate_pty: false,
        }
    }

    pub fn arg(mut self, a: impl Into<OsString>) -> Self {
        self.args.push(a.into());
        self
    }

    pub fn arg_pair(mut self, flag: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.args.push(flag.into());
        self.args.push(value.into());
        self
    }

    pub fn envv(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.env.insert(k.into(), v.into());
        self
    }

    pub fn with_pty(mut self) -> Self {
        self.allocate_pty = true;
        self
    }
}

/// A stream parser owned by the harness for the lifetime of one run.
///
/// Adapters that emit NDJSON typically maintain no state and parse
/// each line independently; adapters that emit multi-line frames can
/// buffer in `self`. `flush` is called once at EOF.
pub trait CliEventParser: Send {
    fn parse_line(&mut self, line: &str) -> Result<Vec<CodingCliEvent>, ParseError>;
    fn flush(&mut self) -> Result<Vec<CodingCliEvent>, ParseError>;
}

/// The integration seam each CLI adapter implements.
#[async_trait]
pub trait CliVendor: Send + Sync {
    /// Stable identifier this adapter answers to.
    fn kind(&self) -> CliVendorKind;

    /// Human-friendly label for the UI (e.g. "Claude Code").
    fn label(&self) -> &str;

    /// Build the command line for a headless run.
    fn build_headless_command(&self, req: &CliRequest, workdir: &Path) -> CliCommand;

    /// Build the command line for an interactive run (TUI).
    fn build_interactive_command(&self, req: &CliRequest, workdir: &Path) -> CliCommand;

    /// Construct a fresh parser for one run's stream of NDJSON / lines.
    fn new_parser(&self) -> Box<dyn CliEventParser>;

    /// Write the vendor's on-disk config (e.g. `CLAUDE.md`,
    /// `.mcp.json`, `AGENTS.md`) from the supplied projection.
    /// Called *before* every run. Idempotent — overwrites prior files
    /// the harness placed there.
    async fn materialize_config(
        &self,
        projection: &ConceptProjection,
        workdir: &Path,
    ) -> Result<(), MapperError>;

    /// Probe whether the CLI is actually installed and runnable in
    /// the current isolator. The harness skips vendors that return
    /// `false` from the `/api/cli/vendors` listing.
    async fn is_available(&self) -> bool {
        true
    }
}
