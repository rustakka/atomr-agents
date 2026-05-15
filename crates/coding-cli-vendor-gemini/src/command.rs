use std::path::Path;

use atomr_agents_coding_cli_core::{CliCommand, CliRequest};

const GEMINI_BIN: &str = "gemini";

pub fn build_headless(req: &CliRequest, workdir: &Path) -> CliCommand {
    let mut cmd = CliCommand::new(GEMINI_BIN, workdir);
    cmd = cmd
        .arg("-p")
        .arg(&req.prompt)
        .arg_pair("--output-format", "stream-json")
        .arg("--non-interactive");
    if let Some(model) = req.model.as_deref() {
        cmd = cmd.arg_pair("--model", model);
    }
    if req.project.policy.auto_approve_unrestricted {
        cmd = cmd.arg("--yolo");
    }
    cmd
}

pub fn build_interactive(req: &CliRequest, workdir: &Path) -> CliCommand {
    let mut cmd = CliCommand::new(GEMINI_BIN, workdir).with_pty();
    if let Some(model) = req.model.as_deref() {
        cmd = cmd.arg_pair("--model", model);
    }
    cmd
}
