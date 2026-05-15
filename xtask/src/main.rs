use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "xtask", about = "atomr-agents workspace tooling")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run an eval suite for a harness version.
    HarnessEval { spec: String },
    /// Replay a recorded event stream.
    Replay { trace: String },
    /// Bump the workspace version.
    Bump { kind: String },
    /// Audit the workspace lint baseline.
    Audit,
    /// Build the stt-harness-web React SPA into `ui/dist` so the crate
    /// can be compiled with `--features embed-ui`.
    SttWebBuild,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Cmd::HarnessEval { spec } => {
            println!("xtask harness-eval {spec} — implemented in Phase 7");
        }
        Cmd::Replay { trace } => {
            println!("xtask replay {trace} — implemented in Phase 7");
        }
        Cmd::Bump { kind } => {
            println!("xtask bump {kind} — implemented in Phase 11");
        }
        Cmd::Audit => {
            println!("xtask audit — implemented in Phase 11");
        }
        Cmd::SttWebBuild => stt_web_build()?,
    }
    Ok(())
}

/// Build the stt-harness-web React SPA: `npm ci` then `npm run build`.
fn stt_web_build() -> anyhow::Result<()> {
    use std::process::Command;

    let ui_dir = "crates/stt-harness-web/ui";
    println!("xtask stt-web-build — building the React SPA in {ui_dir}");
    for (label, args) in [("npm ci", &["ci"][..]), ("npm run build", &["run", "build"][..])] {
        println!("  $ npm {}", args.join(" "));
        let status = Command::new("npm")
            .args(args)
            .current_dir(ui_dir)
            .status()
            .map_err(|e| anyhow::anyhow!("failed to run `{label}`: {e}"))?;
        if !status.success() {
            anyhow::bail!("`{label}` failed with status {status}");
        }
    }
    println!("done — ui/dist is ready; build with `--features embed-ui`.");
    Ok(())
}
