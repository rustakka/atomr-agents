//! Standalone server binary.
//!
//! Boots a [`ChannelHarness`] with the in-memory store and an empty
//! provider set, then serves the REST + WS surface. Useful for
//! smoke-testing the surface and developing the web UI without writing
//! a custom embed.

use std::sync::Arc;

use atomr_agents_channel_harness::ChannelHarness;
use atomr_agents_channel_harness_web::{WebConfig, WebServer};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(7400);
    let mut config = WebConfig::default();
    config.bind = format!("127.0.0.1:{port}").parse()?;

    let harness = Arc::new(ChannelHarness::in_memory());
    let server = WebServer::new(config.clone(), harness);
    let handle = server.start().await?;
    tracing::info!(addr = %handle.bound_addr, "channel-harness-web listening");

    tokio::signal::ctrl_c().await?;
    handle.shutdown().await;
    Ok(())
}
