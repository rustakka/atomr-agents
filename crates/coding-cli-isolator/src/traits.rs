use async_trait::async_trait;

use atomr_agents_coding_cli_core::CliCommand;

use crate::error::IsolatorError;
use crate::handle::{IsolationOpts, ProcessHandle};

/// Spawns a [`CliCommand`] in some execution environment and returns
/// a uniform [`ProcessHandle`].
#[async_trait]
pub trait Isolator: Send + Sync {
    /// Stable identifier used in `/api/cli/vendors` and logs.
    fn name(&self) -> &str;

    async fn spawn(
        &self,
        cmd: CliCommand,
        opts: IsolationOpts,
    ) -> Result<Box<dyn ProcessHandle>, IsolatorError>;
}
