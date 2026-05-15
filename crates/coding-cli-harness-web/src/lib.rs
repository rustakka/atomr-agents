//! Axum + embedded SPA companion for the coding-cli harness.
//!
//! Mirrors `atomr-agents-deep-research-harness-web` — same
//! `WebServer` / `WebHandle` / `WebConfig` / `AppState` split — but
//! adds a WebSocket route for the tmux-PTY bridge in interactive mode.

#![forbid(unsafe_code)]

pub mod error;
pub mod routes;
pub mod runner;
pub mod spa;
pub mod sse;
pub mod ws;

use std::net::SocketAddr;
use std::sync::Arc;

use atomr_agents_coding_cli_harness::CodingCliHarness;
use axum::Router;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::runner::RunSupervisor;

#[derive(Clone, Debug)]
pub struct WebConfig {
    pub bind: SocketAddr,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            // 7000 stt, 7100 meetings, 7200 deep-research → 7300.
            bind: "127.0.0.1:7300".parse().expect("valid default addr"),
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub harness: Arc<CodingCliHarness>,
    pub supervisor: Arc<parking_lot::Mutex<RunSupervisor>>,
}

pub struct WebHandle {
    pub bound_addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
    join: Option<JoinHandle<()>>,
}

impl WebHandle {
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(join) = self.join.take() {
            let _ = join.await;
        }
    }
}

pub struct WebServer {
    config: WebConfig,
    state: AppState,
}

impl WebServer {
    pub fn new(config: WebConfig, harness: Arc<CodingCliHarness>) -> Self {
        Self {
            config,
            state: AppState {
                harness,
                supervisor: Arc::new(parking_lot::Mutex::new(RunSupervisor::default())),
            },
        }
    }

    pub fn router(&self) -> Router {
        routes::build_router(self.state.clone())
    }

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
