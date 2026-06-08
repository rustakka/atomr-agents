//! Standalone sandbox-harness web server. Serves a mock-backed harness by
//! default (set up a Docker/Firecracker-backed harness in code for real exec).

use std::sync::Arc;

use atomr_agents_sandbox_harness::SandboxHarness;
use atomr_agents_sandbox_harness_web::{WebConfig, WebServer};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt::init();

    let harness = Arc::new(SandboxHarness::local_default());
    let config = WebConfig::default();
    let server = WebServer::new(config, harness);
    tracing::info!(addr = %server.bind_addr(), "starting sandbox-harness-web");
    server.serve().await
}
