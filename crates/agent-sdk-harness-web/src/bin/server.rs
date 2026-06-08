//! Dev/demo server for the agent-sdk harness web companion.
//!
//! Serves the in-memory [`MockBackend`](atomr_agents_agent_sdk_core::MockBackend)
//! over REST + SSE — useful for exercising the HTTP surface without the
//! `claude-agent-sdk` / `claude` CLI. Production deployments construct the
//! harness with the Python-driven backend (via `py-bindings`) and call
//! [`WebServer::serve`] directly.

use std::net::SocketAddr;
use std::sync::Arc;

use atomr_agents_agent_sdk_harness::AgentSdkHarness;
use atomr_agents_agent_sdk_harness_web::{WebConfig, WebServer};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt::init();

    let bind: SocketAddr = std::env::var("AGENT_SDK_WEB_BIND")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| SocketAddr::from(([0, 0, 0, 0], 8080)));

    let harness = Arc::new(AgentSdkHarness::local_default());
    tracing::info!(%bind, backend = harness.backend_name(), "agent-sdk web companion listening");

    WebServer::new(WebConfig { bind }, harness).serve().await
}
