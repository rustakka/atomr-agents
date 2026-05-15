//! Stand-alone binary serving the embedded coding-cli SPA + REST + SSE
//! + WebSocket endpoints with the default `CodingCliHarness::local_default()`.

use std::sync::Arc;

use atomr_agents_coding_cli_harness::CodingCliHarness;
use atomr_agents_coding_cli_harness_web::{WebConfig, WebServer};

#[tokio::main]
async fn main() -> anyhow_local::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let harness = Arc::new(CodingCliHarness::local_default());
    let cfg = WebConfig::default();
    tracing::info!(bind = %cfg.bind, "starting coding-cli web companion");
    let server = WebServer::new(cfg, harness);
    let handle = server
        .start()
        .await
        .map_err(|e| anyhow_local::anyhow!("bind failed: {e}"))?;
    tracing::info!(addr = %handle.bound_addr, "listening");
    // Wait for SIGINT.
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("ctrl-c received, shutting down");
    handle.shutdown().await;
    Ok(())
}

// Minimal local anyhow shim so we don't need to add the real anyhow
// crate to dependencies for one main.rs.
mod anyhow_local {
    pub type Result<T> = std::result::Result<T, Error>;
    #[derive(Debug)]
    pub struct Error(pub String);
    impl std::fmt::Display for Error {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.0)
        }
    }
    impl std::error::Error for Error {}
    #[macro_export]
    macro_rules! anyhow_local_macro {
        ($($t:tt)*) => { $crate::anyhow_local::Error(format!($($t)*)) };
    }
    pub use crate::anyhow_local_macro as anyhow;
}
