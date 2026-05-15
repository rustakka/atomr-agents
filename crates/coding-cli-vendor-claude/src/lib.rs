//! Claude Code adapter for the coding-cli harness.

#![forbid(unsafe_code)]

mod command;
mod mapper;
mod parser;

use std::path::Path;

use async_trait::async_trait;

use atomr_agents_coding_cli_core::{
    CliCommand, CliEventParser, CliRequest, CliVendor, CliVendorKind, ConceptProjection,
    MapperError,
};

pub use command::{build_headless, build_interactive};
pub use mapper::materialize as materialize_claude_config;
pub use parser::ClaudeParser;

/// The Claude Code adapter.
#[derive(Debug, Clone, Default)]
pub struct ClaudeVendor;

impl ClaudeVendor {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl CliVendor for ClaudeVendor {
    fn kind(&self) -> CliVendorKind {
        CliVendorKind::Claude
    }

    fn label(&self) -> &str {
        "Claude Code"
    }

    fn build_headless_command(&self, req: &CliRequest, workdir: &Path) -> CliCommand {
        build_headless(req, workdir)
    }

    fn build_interactive_command(&self, req: &CliRequest, workdir: &Path) -> CliCommand {
        build_interactive(req, workdir)
    }

    fn new_parser(&self) -> Box<dyn CliEventParser> {
        Box::new(ClaudeParser::new())
    }

    async fn materialize_config(
        &self,
        projection: &ConceptProjection,
        workdir: &Path,
    ) -> Result<(), MapperError> {
        materialize_claude_config(projection, workdir).await
    }

    async fn is_available(&self) -> bool {
        which("claude").await
    }
}

async fn which(bin: &str) -> bool {
    tokio::process::Command::new("which")
        .arg(bin)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}
