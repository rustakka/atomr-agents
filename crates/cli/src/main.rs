mod inspector;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "atomr-agents",
    version,
    about = "Operate the atomr-agents registry, harnesses, and eval suites."
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Eval-suite operations.
    Eval {
        #[command(subcommand)]
        op: EvalCmd,
    },
    /// Registry operations.
    Registry {
        #[command(subcommand)]
        op: RegistryCmd,
    },
    /// Harness lifecycle.
    Harness {
        #[command(subcommand)]
        op: HarnessCmd,
    },
    /// Studio-style inspector (read+resume) — print supported endpoints.
    Serve {
        #[arg(long, default_value = "127.0.0.1:7000")]
        bind: String,
    },
}

#[derive(Subcommand)]
enum EvalCmd {
    /// Run an eval suite for a harness id@version.
    Run {
        /// Harness specification, e.g. `coding@0.1.0`.
        spec: String,
    },
    /// Compare a current run to a stored baseline.
    Gate {
        baseline_path: String,
        current_path: String,
        #[arg(long, default_value_t = 0.05)]
        tolerance: f32,
    },
}

#[derive(Subcommand)]
enum RegistryCmd {
    /// List artifacts of a kind.
    List { kind: String },
    /// Show a specific artifact.
    Get {
        kind: String,
        id: String,
        version: String,
    },
}

#[derive(Subcommand)]
enum HarnessCmd {
    /// Pin a harness id@version into the local registry. Stub: prints
    /// the resolved spec.
    Pin { spec: String },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let cli = Cli::parse();
    match cli.command {
        Cmd::Eval { op } => match op {
            EvalCmd::Run { spec } => {
                println!("eval run {spec}: load harness, run eval suite, write result.");
                println!(
                    "  (stub — integrates `atomr_agents_eval::EvalSuite` when registry-backed harnesses land)"
                );
            }
            EvalCmd::Gate { baseline_path, current_path, tolerance } => {
                println!(
                    "eval gate baseline={baseline_path} current={current_path} tol={tolerance}"
                );
                println!(
                    "  (stub — call `RegressionGate::check` after deserializing the EvalRun JSONs)"
                );
            }
        },
        Cmd::Registry { op } => match op {
            RegistryCmd::List { kind } => {
                println!("registry list {kind}");
                println!(
                    "  (stub — backing store wiring lands when persistence-backed registry plugs in)"
                );
            }
            RegistryCmd::Get { kind, id, version } => {
                println!("registry get {kind} {id} {version}");
            }
        },
        Cmd::Harness { op } => match op {
            HarnessCmd::Pin { spec } => {
                println!("harness pin {spec}");
            }
        },
        Cmd::Serve { bind } => {
            println!("atomr-agents Studio-style inspector — would listen on {bind}");
            println!("Endpoints (handler functions in `inspector::Inspector`):");
            println!("  GET  /runs/:wf/:run/checkpoints");
            println!("  GET  /runs/:wf/:run/checkpoints/:step");
            println!("  POST /runs/:wf/:run/fork");
            println!("  POST /runs/:wf/:run/resume");
            println!("Bind axum onto these (the inspector module is feature-flag-friendly).");
        }
    }
    Ok(())
}
