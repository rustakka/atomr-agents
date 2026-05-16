//! Optional web companion for the channel harness.
//!
//! An axum server that:
//!
//! - Receives provider webhooks at `POST /webhook/<provider>/<channel_id>`
//!   and forwards verified payloads into [`ChannelHarness::ingest_webhook`].
//! - Exposes REST CRUD over channels / threads / messages.
//! - Streams [`ChannelEvent`]s on `GET /ws`.
//!
//! Default bind: `127.0.0.1:7400` — chosen to compose alongside the
//! existing `*-harness-web` servers (`stt:7000`, `meetings:7100`,
//! `coding-cli:7200`, `deep-research:7300`).
//!
//! Mirrors `atomr-agents-meetings-harness-web` for the
//! [`WebConfig`] / [`AppState`] / [`WebHandle`] / [`WebServer`] split.

#![forbid(unsafe_code)]

pub mod routes;
pub mod ws;

use std::net::SocketAddr;
use std::sync::Arc;

use atomr_agents_channel_core::ChannelEvent;
use atomr_agents_channel_harness::ChannelHarness;
use axum::Router;
use tokio::sync::{broadcast, oneshot};
use tokio::task::JoinHandle;

/// Configuration for the web server.
#[derive(Clone, Debug)]
pub struct WebConfig {
    pub bind: SocketAddr,
    pub ws_channel_capacity: usize,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:7400".parse().expect("valid default addr"),
            ws_channel_capacity: 512,
        }
    }
}

/// Shared router state.
#[derive(Clone)]
pub struct AppState {
    pub harness: Arc<ChannelHarness>,
    pub events: broadcast::Sender<ChannelEvent>,
}

/// Running server handle.
pub struct WebHandle {
    pub bound_addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
    join: Option<JoinHandle<()>>,
    forwarder: Option<JoinHandle<()>>,
}

impl WebHandle {
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(join) = self.join.take() {
            let _ = join.await;
        }
        if let Some(forwarder) = self.forwarder.take() {
            forwarder.abort();
            let _ = forwarder.await;
        }
    }
}

/// The web server.
pub struct WebServer {
    config: WebConfig,
    state: AppState,
}

impl WebServer {
    pub fn new(config: WebConfig, harness: Arc<ChannelHarness>) -> Self {
        let (events, _) = broadcast::channel(config.ws_channel_capacity);
        Self {
            config,
            state: AppState { harness, events },
        }
    }

    /// Build the axum router. Public so tests can drive handlers without
    /// binding a socket.
    pub fn router(&self) -> Router {
        routes::build_router(self.state.clone())
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// Bind and start serving.
    pub async fn start(self) -> Result<WebHandle, ServerError> {
        let router = self.router();
        let listener = tokio::net::TcpListener::bind(self.config.bind)
            .await
            .map_err(ServerError::Bind)?;
        let bound_addr = listener.local_addr().map_err(ServerError::Bind)?;
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        // Bridge harness events into the broadcast for `/ws` consumers.
        let mut stream = self.state.harness.events();
        let sink = self.state.events.clone();
        let forwarder = tokio::spawn(async move {
            while let Some(ev) = stream.recv().await {
                let _ = sink.send(ev);
            }
        });

        let join = tokio::spawn(async move {
            let _ = axum::serve(listener, router.into_make_service())
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await;
        });
        Ok(WebHandle {
            bound_addr,
            shutdown_tx: Some(shutdown_tx),
            join: Some(join),
            forwarder: Some(forwarder),
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("failed to bind: {0}")]
    Bind(std::io::Error),
}
