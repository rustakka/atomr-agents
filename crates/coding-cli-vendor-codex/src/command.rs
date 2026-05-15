use std::path::Path;

use atomr_agents_coding_cli_core::{CliCommand, CliRequest};

const CODEX_BIN: &str = "codex";

pub fn build_headless(req: &CliRequest, workdir: &Path) -> CliCommand {
    let mut cmd = CliCommand::new(CODEX_BIN, workdir).arg("exec").arg(&req.prompt);
    if let Some(model) = req.model.as_deref() {
        cmd = cmd.arg_pair("--model", model);
    }
    if req.project.policy.auto_approve_unrestricted {
        cmd = cmd.arg("--full-access");
    }
    cmd
}

pub fn build_interactive(req: &CliRequest, workdir: &Path) -> CliCommand {
    let mut cmd = CliCommand::new(CODEX_BIN, workdir).with_pty();
    if let Some(model) = req.model.as_deref() {
        cmd = cmd.arg_pair("--model", model);
    }
    cmd
}
