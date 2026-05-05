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
    }
    Ok(())
}
