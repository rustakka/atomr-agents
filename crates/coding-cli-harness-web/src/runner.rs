//! Tracks the background tokio tasks driving in-flight headless runs.

use std::collections::HashMap;

use atomr_agents_coding_cli_core::CliRunId;
use tokio::task::JoinHandle;

#[derive(Default)]
pub struct RunSupervisor {
    pub active: HashMap<CliRunId, JoinHandle<()>>,
}

impl RunSupervisor {
    pub fn register(&mut self, id: CliRunId, task: JoinHandle<()>) {
        self.active.insert(id, task);
    }

    pub fn forget(&mut self, id: &CliRunId) {
        self.active.remove(id);
    }
}
