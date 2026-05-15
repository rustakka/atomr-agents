//! OpenAI Codex CLI adapter.

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
pub use mapper::materialize as materialize_codex_config;
pub use parser::CodexParser;

#[derive(Debug, Clone, Default)]
pub struct CodexVendor;

impl CodexVendor {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl CliVendor for CodexVendor {
    fn kind(&self) -> CliVendorKind {
        CliVendorKind::Codex
    }

    fn label(&self) -> &str {
        "OpenAI Codex"
    }

    fn build_headless_command(&self, req: &CliRequest, workdir: &Path) -> CliCommand {
        build_headless(req, workdir)
    }

    fn build_interactive_command(&self, req: &CliRequest, workdir: &Path) -> CliCommand {
        build_interactive(req, workdir)
    }

    fn new_parser(&self) -> Box<dyn CliEventParser> {
        Box::new(CodexParser::new())
    }

    async fn materialize_config(
        &self,
        projection: &ConceptProjection,
        workdir: &Path,
    ) -> Result<(), MapperError> {
        materialize_codex_config(projection, workdir).await
    }

    async fn is_available(&self) -> bool {
        tokio::process::Command::new("which")
            .arg("codex")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}
