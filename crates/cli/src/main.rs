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

#[tokio::main]
async fn main() -> Result<()> {
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
            EvalCmd::Gate {
                baseline_path,
                current_path,
                tolerance,
            } => {
                println!("eval gate baseline={baseline_path} current={current_path} tol={tolerance}");
                println!("  (stub — call `RegressionGate::check` after deserializing the EvalRun JSONs)");
            }
        },
        Cmd::Registry { op } => match op {
            RegistryCmd::List { kind } => {
                println!("registry list {kind}");
                println!("  (stub — backing store wiring lands when persistence-backed registry plugs in)");
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
        Cmd::Serve { bind } => serve(bind).await?,
    }
    Ok(())
}

/// `atomr-agents serve` — run the STT-harness conversation review UI.
///
/// Compiled in only with `--features stt-web`. Conversations are
/// persisted through `crates/state`'s `Checkpointer` — the configured
/// persistence provider — defaulting to the in-memory backend.
#[cfg(feature = "stt-web")]
async fn serve(bind: String) -> Result<()> {
    use std::net::SocketAddr;
    use std::sync::Arc;

    use atomr_agents_state::{Checkpointer, InMemoryCheckpointer};
    use atomr_agents_stt_harness::{CheckpointerConversationStore, ConversationStore};
    use atomr_agents_stt_harness_web::{WebConfig, WebServer};

    let addr: SocketAddr = bind
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid --bind '{bind}': {e}"))?;

    // The configured persistence provider. `InMemoryCheckpointer` is
    // the default; a deployment can swap in the SQLite/Postgres backend
    // via `crates/state`'s feature flags.
    let checkpointer: Arc<dyn Checkpointer> = Arc::new(InMemoryCheckpointer::new());
    let store: Arc<dyn ConversationStore> = Arc::new(CheckpointerConversationStore::new(checkpointer));

    let server = WebServer::new(
        WebConfig {
            bind: addr,
            ws_channel_capacity: 512,
        },
        store,
    );
    let handle = server.start().await?;
    println!("stt-harness-web listening on http://{}", handle.bound_addr);
    println!("  GET    /api/conversations");
    println!("  GET    /api/conversations/:id");
    println!("  PUT    /api/conversations/:id/speakers/:speaker_id");
    println!("  DELETE /api/conversations/:id");
    println!("  GET    /ws    (live SttHarnessEvent stream)");
    println!("Press Ctrl-C to stop.");
    tokio::signal::ctrl_c().await.ok();
    println!("shutting down…");
    handle.shutdown().await;
    Ok(())
}

/// Fallback when the web UI is not compiled in.
#[cfg(not(feature = "stt-web"))]
async fn serve(bind: String) -> Result<()> {
    println!("atomr-agents serve — the STT conversation review UI is not compiled in.");
    println!("Rebuild with `--features stt-web` to serve it on {bind}:");
    println!("  GET    /api/conversations");
    println!("  GET    /api/conversations/:id");
    println!("  PUT    /api/conversations/:id/speakers/:speaker_id");
    println!("  GET    /ws");
    println!("(The Studio-style checkpoint inspector — see `inspector::Inspector` — remains TODO.)");
    Ok(())
}
