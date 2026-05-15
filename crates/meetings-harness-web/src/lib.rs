//! Optional web backend for the meetings harness.
//!
//! An axum server exposing a REST + WebSocket surface over a
//! [`MeetingsStore`], plus (with the `embed-ui` feature) the embedded
//! React SPA for reviewing meeting analyses. Mirrors
//! `atomr-agents-stt-harness-web`: same [`WebServer`] / [`WebHandle`] /
//! [`WebConfig`] / [`AppState`] split, runs on port `7100` by default
//! so it composes side-by-side with the STT web UI on `7000`.

#![forbid(unsafe_code)]

pub mod routes;
pub mod runner;
pub mod spa;
pub mod ws;

use std::net::SocketAddr;
use std::sync::Arc;

use atomr_agents_meetings_harness::{MeetingsHarnessEvent, MeetingsStore};
use atomr_agents_stt_harness::ConversationStore;
use axum::Router;
use parking_lot::Mutex;
use tokio::sync::{broadcast, oneshot};
use tokio::task::JoinHandle;

use crate::runner::RunSupervisor;

/// Configuration for the web server.
#[derive(Clone, Debug)]
pub struct WebConfig {
    pub bind: SocketAddr,
    pub ws_channel_capacity: usize,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:7100".parse().expect("valid default addr"),
            ws_channel_capacity: 512,
        }
    }
}

/// Shared router state.
#[derive(Clone)]
pub struct AppState {
    /// The meeting-analysis persistence backend.
    pub store: Arc<dyn MeetingsStore>,
    /// The STT-side conversation store — needed so the dashboard can
    /// list available source transcripts and look them up by id when a
    /// run is triggered.
    pub transcripts: Arc<dyn ConversationStore>,
    /// Fan-out channel of live meetings events for the `/ws` route.
    pub events: broadcast::Sender<MeetingsHarnessEvent>,
    /// Supervisor over the most recent in-flight run (live or batch),
    /// holds a cancellation hook so `POST /api/meetings/:id/stop` can
    /// signal it.
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
    pub fn new(
        config: WebConfig,
        store: Arc<dyn MeetingsStore>,
        transcripts: Arc<dyn ConversationStore>,
    ) -> Self {
        let (events, _) = broadcast::channel(config.ws_channel_capacity);
        Self {
            config,
            state: AppState {
                store,
                transcripts,
                events,
                supervisor: Arc::new(Mutex::new(RunSupervisor::default())),
            },
        }
    }

    /// Broadcast sender for live meetings events.
    pub fn event_sender(&self) -> broadcast::Sender<MeetingsHarnessEvent> {
        self.state.events.clone()
    }

    /// Build the axum router. Public so tests can drive handlers
    /// through `tower::ServiceExt::oneshot` without binding a socket.
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
