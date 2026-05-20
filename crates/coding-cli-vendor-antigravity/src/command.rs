use std::path::Path;

use atomr_agents_coding_cli_core::{CliCommand, CliRequest};

/// Configurable invocation surface for the Antigravity CLI (`agy`).
///
/// `agy`'s headless flags are not yet fully documented publicly, so the
/// binary name, flags, and on-disk config layout are all overridable.
/// The defaults mirror the legacy Gemini CLI surface; an operator can
/// correct any field without a code change once `agy --help` is known.
#[derive(Debug, Clone)]
pub struct AntigravityConfig {
    pub binary: String,
    pub prompt_flag: String,
    pub output_format_flag: String,
    pub output_format: String,
    pub non_interactive_flag: String,
    pub model_flag: String,
    pub yolo_flag: String,
    pub config_dir: String,
    pub settings_file: String,
    pub system_instructions_file: String,
}

impl Default for AntigravityConfig {
    fn default() -> Self {
        // TODO(agy): verify flag names + config dir against `agy --help`.
        Self {
            binary: "agy".into(),
            prompt_flag: "-p".into(),
            output_format_flag: "--output-format".into(),
            output_format: "stream-json".into(),
            non_interactive_flag: "--non-interactive".into(),
            model_flag: "--model".into(),
            yolo_flag: "--yolo".into(),
            config_dir: ".antigravity".into(),
            settings_file: "settings.json".into(),
            system_instructions_file: "system_instructions.md".into(),
        }
    }
}

pub fn build_headless(req: &CliRequest, workdir: &Path, cfg: &AntigravityConfig) -> CliCommand {
    let mut cmd = CliCommand::new(&cfg.binary, workdir);
    cmd = cmd
        .arg(&cfg.prompt_flag)
        .arg(&req.prompt)
        .arg_pair(&cfg.output_format_flag, &cfg.output_format)
        .arg(&cfg.non_interactive_flag);
    // `req.model` reaches `agy` here — the path by which a non-Gemini
    // model id (e.g. a Claude model) is selected via the model flag.
    if let Some(model) = req.model.as_deref() {
        cmd = cmd.arg_pair(&cfg.model_flag, model);
    }
    if req.project.policy.auto_approve_unrestricted {
        cmd = cmd.arg(&cfg.yolo_flag);
    }
    cmd
}

pub fn build_interactive(req: &CliRequest, workdir: &Path, cfg: &AntigravityConfig) -> CliCommand {
    let mut cmd = CliCommand::new(&cfg.binary, workdir).with_pty();
    if let Some(model) = req.model.as_deref() {
        cmd = cmd.arg_pair(&cfg.model_flag, model);
    }
    cmd
}
