use std::path::Path;

use atomr_agents_coding_cli_core::{CliCommand, CliRequest};

const CLAUDE_BIN: &str = "claude";

/// Build a non-interactive `claude -p` invocation with stream-json
/// output.
pub fn build_headless(req: &CliRequest, workdir: &Path) -> CliCommand {
    let mut cmd = CliCommand::new(CLAUDE_BIN, workdir);
    cmd = cmd
        .arg("-p")
        .arg(&req.prompt)
        .arg_pair("--output-format", "stream-json")
        .arg("--verbose")
        .arg("--include-partial-messages");

    if let Some(model) = req.model.as_deref() {
        cmd = cmd.arg_pair("--model", model);
    }
    if !req.allowed_tools.is_empty() {
        cmd = cmd.arg_pair("--allowed-tools", req.allowed_tools.join(","));
    }
    if let Some(session) = req.resume_session.as_deref() {
        cmd = cmd.arg_pair("--resume", session);
    }

    if req.project.policy.auto_approve_unrestricted {
        // permissive mode for trusted CI; the user opted in.
        cmd = cmd.arg_pair("--permission-mode", "acceptEdits");
    }
    cmd
}

/// Build the `claude` interactive TUI invocation (no `-p`).
pub fn build_interactive(req: &CliRequest, workdir: &Path) -> CliCommand {
    let mut cmd = CliCommand::new(CLAUDE_BIN, workdir).with_pty();
    if let Some(model) = req.model.as_deref() {
        cmd = cmd.arg_pair("--model", model);
    }
    if let Some(session) = req.resume_session.as_deref() {
        cmd = cmd.arg_pair("--resume", session);
    }
    cmd
}
