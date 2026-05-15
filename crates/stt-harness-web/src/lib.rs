//! Optional web backend for the STT harness.
//!
//! An axum server exposing a REST + WebSocket surface over a
//! [`ConversationStore`], plus (with the `embed-ui` feature) the
//! embedded React SPA for reviewing diarized transcripts and editing
//! speaker labels. Structurally mirrors `atomr-dashboard`:
//! [`WebServer`] / [`WebHandle`] / [`WebConfig`] / [`AppState`], with
//! [`WebServer::router`] public so tests can drive handlers through
//! `tower::ServiceExt::oneshot` without binding a socket.
//!
//! ```no_run
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! use std::sync::Arc;
//! use atomr_agents_stt_harness::InMemoryConversationStore;
//! use atomr_agents_stt_harness_web::{WebConfig, WebServer};
//!
//! let store = Arc::new(InMemoryConversationStore::new());
//! let server = WebServer::new(WebConfig::default(), store);
//! // Forward an `SttHarness`'s events into the WebSocket stream:
//! // tokio::spawn(forward(harness.events(), server.event_sender()));
//! let handle = server.start().await?;
//! handle.shutdown().await;
//! # Ok(()) }
//! ```

#![forbid(unsafe_code)]

pub mod routes;
pub mod spa;
pub mod ws;

use std::net::SocketAddr;
use std::sync::Arc;

use atomr_agents_stt_harness::{ConversationStore, SttHarnessEvent};
use axum::Router;
use tokio::sync::{broadcast, oneshot};
use tokio::task::JoinHandle;

/// Configuration for the web server.
#[derive(Clone, Debug)]
pub struct WebConfig {
    /// Address to bind.
    pub bind: SocketAddr,
    /// Buffer size for the WebSocket fan-out channel.
    pub ws_channel_capacity: usize,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:7000".parse().expect("valid default addr"),
            ws_channel_capacity: 512,
        }
    }
}

/// Shared router state. Cloned into every handler.
#[derive(Clone)]
pub struct AppState {
    /// The conversation persistence backend.
    pub store: Arc<dyn ConversationStore>,
    /// Fan-out channel of live STT-harness events for the `/ws` route.
    pub events: broadcast::Sender<SttHarnessEvent>,
}

/// Running server handle. Drop to leave it running; call
/// [`WebHandle::shutdown`] to stop gracefully.
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

/// The web server — holds config + shared state, builds the router,
/// and binds.
pub struct WebServer {
    config: WebConfig,
    state: AppState,
}

impl WebServer {
    /// Build a server over the given conversation store.
    pub fn new(config: WebConfig, store: Arc<dyn ConversationStore>) -> Self {
        let (events, _) = broadcast::channel(config.ws_channel_capacity);
        Self {
            config,
            state: AppState { store, events },
        }
    }

    /// The broadcast sender for live harness events. Forward an
    /// `SttHarness`'s event stream into this so the `/ws` route can
    /// relay it to browsers.
    pub fn event_sender(&self) -> broadcast::Sender<SttHarnessEvent> {
        self.state.events.clone()
    }

    /// Build the axum router. Public so tests can exercise handlers
    /// via `tower::ServiceExt::oneshot` without binding a socket.
    pub fn router(&self) -> Router {
        routes::build_router(self.state.clone())
    }

    /// Bind and start serving. Returns once the listener is bound.
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

/// Errors raised while starting the server.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("failed to bind: {0}")]
    Bind(std::io::Error),
}
