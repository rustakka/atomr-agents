//! Axum + embedded SPA companion for the deep-research harness.
//!
//! Mirrors `atomr-agents-meetings-harness-web` — same
//! `WebServer` / `WebHandle` / `WebConfig` / `AppState` split — but
//! exposes SSE for live event streaming and a slimmer embedded SPA
//! (vanilla HTML/JS) so the crate doesn't pull in a Node toolchain.

#![forbid(unsafe_code)]

pub mod routes;
pub mod runner;
pub mod spa;
pub mod sse;

use std::net::SocketAddr;
use std::sync::Arc;

use atomr_agents_deep_research_harness::{DeepResearchEvent, ResearchStore};
use atomr_agents_web_search_core::WebSearch;
use axum::Router;
use parking_lot::Mutex;
use tokio::sync::{broadcast, oneshot};
use tokio::task::JoinHandle;

use crate::runner::RunSupervisor;

/// Configuration for the web server.
#[derive(Clone, Debug)]
pub struct WebConfig {
    pub bind: SocketAddr,
    pub sse_channel_capacity: usize,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            // 7100 is meetings, 7000 is stt, 7200 = deep-research.
            bind: "127.0.0.1:7200".parse().expect("valid default addr"),
            sse_channel_capacity: 512,
        }
    }
}

/// Shared router state.
#[derive(Clone)]
pub struct AppState {
    /// The research-result persistence backend.
    pub store: Arc<dyn ResearchStore>,
    /// Web-search provider used by spawned runs. Pluggable so callers
    /// can wire a `MockWebSearch` in tests and a real provider in prod.
    pub search: Arc<dyn WebSearch>,
    /// Fan-out channel of live events for SSE consumers.
    pub events: broadcast::Sender<DeepResearchEvent>,
    /// Supervisor over the most-recent in-flight run.
    pub supervisor: Arc<Mutex<RunSupervisor>>,
}

/// Running server handle.
pub struct WebHandle {
    pub bound_addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
    join: Option<JoinHandle<()>>,
}

impl WebHandle {
    /// Signal graceful shutdown and wait for the server task to exit.
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(join) = self.join.take() {
            let _ = join.await;
        }
    }
}

/// The web server.
pub struct WebServer {
    config: WebConfig,
    state: AppState,
}

impl WebServer {
    pub fn new(config: WebConfig, store: Arc<dyn ResearchStore>, search: Arc<dyn WebSearch>) -> Self {
        let (events, _) = broadcast::channel(config.sse_channel_capacity);
        Self {
            config,
            state: AppState {
                store,
                search,
                events,
                supervisor: Arc::new(Mutex::new(RunSupervisor::default())),
            },
        }
    }

    /// Broadcast sender for live events.
    pub fn event_sender(&self) -> broadcast::Sender<DeepResearchEvent> {
        self.state.events.clone()
    }

    /// Build the axum router (exposed so tests can drive handlers via
    /// `tower::ServiceExt::oneshot`).
    pub fn router(&self) -> Router {
        routes::build_router(self.state.clone())
    }

    /// Bind and start serving.
    pub async fn start(self) -> Result<WebHandle, ServerError> {
        let router = self.router();
        let listener = tokio::net::TcpListener::bind(self.config.bind)
            .await
            .map_err(ServerError::Bind)?;
        let bound_addr = listener.local_addr().map_err(ServerError::Bind)?;
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
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
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("failed to bind: {0}")]
    Bind(std::io::Error),
}
