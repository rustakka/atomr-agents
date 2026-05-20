//! Stand-alone binary serving the embedded host SPA + REST + SSE over the
//! default host root (`$ATOMR_HOST_ROOT` or `~/.atomr/host`).

use atomr_agents_host::{HostConfig, HostRuntime};
use atomr_agents_host_web::{WebConfig, WebServer};

#[tokio::main]
async fn main() -> anyhow_local::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = HostConfig::load_default()
        .map_err(|e| anyhow_local::anyhow!("load host config: {e}"))?;
    tracing::info!(root = %config.paths.root.display(), "host root");
    let runtime = HostRuntime::start(config)
        .await
        .map_err(|e| anyhow_local::anyhow!("start runtime: {e}"))?;

    let cfg = WebConfig::default();
    tracing::info!(bind = %cfg.bind, "starting host web companion");
    let server = WebServer::new(cfg, runtime);
    let handle = server
        .start()
        .await
        .map_err(|e| anyhow_local::anyhow!("bind failed: {e}"))?;
    tracing::info!(addr = %handle.bound_addr, "listening");

    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("ctrl-c received, shutting down");
    handle.shutdown().await;
    Ok(())
}

// Minimal local anyhow shim so we don't add the real anyhow crate for one
// main.rs (mirrors the coding-cli-harness-web server binary).
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
