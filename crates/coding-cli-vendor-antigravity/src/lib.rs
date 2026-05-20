//! Google Antigravity CLI (`agy`) adapter. Successor to the Gemini CLI
//! adapter; serves non-Gemini models (Claude, etc.) via the model flag.

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

pub use command::{build_headless, build_interactive, AntigravityConfig};
pub use mapper::materialize as materialize_antigravity_config;
pub use parser::AntigravityParser;

#[derive(Debug, Clone, Default)]
pub struct AntigravityVendor {
    config: AntigravityConfig,
}

impl AntigravityVendor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_config(config: AntigravityConfig) -> Self {
        Self { config }
    }
}

impl From<AntigravityConfig> for AntigravityVendor {
    fn from(config: AntigravityConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl CliVendor for AntigravityVendor {
    fn kind(&self) -> CliVendorKind {
        CliVendorKind::Antigravity
    }

    fn label(&self) -> &str {
        "Antigravity CLI"
    }

    fn build_headless_command(&self, req: &CliRequest, workdir: &Path) -> CliCommand {
        build_headless(req, workdir, &self.config)
    }

    fn build_interactive_command(&self, req: &CliRequest, workdir: &Path) -> CliCommand {
        build_interactive(req, workdir, &self.config)
    }

    fn new_parser(&self) -> Box<dyn CliEventParser> {
        Box::new(AntigravityParser::new())
    }

    async fn materialize_config(
        &self,
        projection: &ConceptProjection,
        workdir: &Path,
    ) -> Result<(), MapperError> {
        materialize_antigravity_config(projection, workdir, &self.config).await
    }

    async fn is_available(&self) -> bool {
        tokio::process::Command::new("which")
            .arg(&self.config.binary)
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}
