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
    /// Meetings harness operations.
    Meetings {
        #[command(subcommand)]
        op: MeetingsCmd,
    },
}

#[derive(Subcommand)]
enum MeetingsCmd {
    /// Analyze an existing STT transcript by `conversation_id`.
    /// Requires `--features meetings`.
    Analyze {
        /// Source transcript id (the `SttConversation::id`).
        #[arg(long)]
        conversation_id: String,
        /// LLM model id. Recorded on the resulting analysis.
        #[arg(long)]
        model: String,
        /// `batch` (default) or `live` (live requires an STT event channel — CLI does not yet wire one).
        #[arg(long, default_value = "batch")]
        mode: String,
        /// Cap on extractor iterations.
        #[arg(long, default_value_t = 32)]
        max_iterations: u32,
    },
    /// Serve the meetings review web UI. Requires `--features meetings-web`.
    Serve {
        #[arg(long, default_value = "127.0.0.1:7100")]
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
        Cmd::Meetings { op } => match op {
            MeetingsCmd::Analyze {
                conversation_id,
                model,
                mode,
                max_iterations,
            } => meetings_analyze(conversation_id, model, mode, max_iterations).await?,
            MeetingsCmd::Serve { bind } => meetings_serve(bind).await?,
        },
    }
    Ok(())
}

#[cfg(feature = "meetings")]
async fn meetings_analyze(
    conversation_id: String,
    model: String,
    mode: String,
    max_iterations: u32,
) -> Result<()> {
    use std::sync::Arc;

    use atomr_agents_meetings_harness::{
        BatchExtractionLoop, CheckpointerMeetingsStore, IterationCapTermination, MeetingsHarness,
        MeetingsHarnessSpec, RuleBasedExtractor, RunMode,
    };
    use atomr_agents_state::{Checkpointer, InMemoryCheckpointer};
    use atomr_agents_stt_harness::{CheckpointerConversationStore, ConversationStore};

    let checkpointer: Arc<dyn Checkpointer> = Arc::new(InMemoryCheckpointer::new());
    let transcripts: Arc<dyn ConversationStore> =
        Arc::new(CheckpointerConversationStore::new(checkpointer.clone()));
    let analyses = Arc::new(CheckpointerMeetingsStore::new(checkpointer));

    let run_mode = match mode.as_str() {
        "batch" => RunMode::Batch,
        "live" => RunMode::Live {
            segment_turn_count: 8,
        },
        other => anyhow::bail!("unknown --mode `{other}` (expected batch or live)"),
    };

    let spec = MeetingsHarnessSpec::new("meetings", model)
        .with_mode(run_mode)
        .with_max_iterations(max_iterations);
    let extractor = Arc::new(RuleBasedExtractor::new());
    let harness = MeetingsHarness::new(
        spec,
        transcripts,
        analyses,
        extractor,
        BatchExtractionLoop,
        IterationCapTermination::new(max_iterations),
    );
    let analysis = harness.run(&conversation_id).await?;
    println!("{}", serde_json::to_string_pretty(&analysis)?);
    Ok(())
}

#[cfg(not(feature = "meetings"))]
async fn meetings_analyze(
    _conversation_id: String,
    _model: String,
    _mode: String,
    _max_iterations: u32,
) -> Result<()> {
    println!(
        "atomr-agents meetings analyze — meetings harness not compiled in.\n\
         Rebuild with `--features meetings` (or `--features meetings-web` to also serve the UI)."
    );
    Ok(())
}

#[cfg(feature = "meetings-web")]
async fn meetings_serve(bind: String) -> Result<()> {
    use std::net::SocketAddr;
    use std::sync::Arc;

    use atomr_agents_meetings_harness::{CheckpointerMeetingsStore, MeetingsStore};
    use atomr_agents_meetings_harness_web::{WebConfig, WebServer};
    use atomr_agents_state::{Checkpointer, InMemoryCheckpointer};
    use atomr_agents_stt_harness::{CheckpointerConversationStore, ConversationStore};

    let addr: SocketAddr = bind
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid --bind '{bind}': {e}"))?;
    let checkpointer: Arc<dyn Checkpointer> = Arc::new(InMemoryCheckpointer::new());
    let transcripts: Arc<dyn ConversationStore> =
        Arc::new(CheckpointerConversationStore::new(checkpointer.clone()));
    let analyses: Arc<dyn MeetingsStore> = Arc::new(CheckpointerMeetingsStore::new(checkpointer));
    let server = WebServer::new(
        WebConfig {
            bind: addr,
            ws_channel_capacity: 512,
        },
        analyses,
        transcripts,
    );
    let handle = server.start().await?;
    println!("meetings-harness-web listening on http://{}", handle.bound_addr);
    println!("  GET    /api/meetings");
    println!("  GET    /api/meetings/:id");
    println!("  PUT    /api/meetings/:id/attendees/:attendee_id");
    println!("  PATCH  /api/meetings/:id/actions/:action_id");
    println!("  POST   /api/meetings/:id/run");
    println!("  POST   /api/meetings/:id/stop");
    println!("  DELETE /api/meetings/:id");
    println!("  GET    /api/transcripts");
    println!("  GET    /ws    (live MeetingsHarnessEvent stream)");
    println!("Press Ctrl-C to stop.");
    tokio::signal::ctrl_c().await.ok();
    println!("shutting down…");
    handle.shutdown().await;
    Ok(())
}

#[cfg(not(feature = "meetings-web"))]
async fn meetings_serve(bind: String) -> Result<()> {
    println!("atomr-agents meetings serve — meetings web UI not compiled in.");
    println!("Rebuild with `--features meetings-web` to serve it on {bind}.");
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
