//! Axum + embedded SPA companion for the `atomr-agents-host` runtime.
//!
//! Mirrors the `atomr-agents-coding-cli-harness-web` shape — same
//! `WebServer` / `WebHandle` / `WebConfig` / `AppState` split — but exposes
//! the full host concept surface (agents, SOUL/MEMORY/RULES/USER docs,
//! skills, curator, crons, hooks, channels/routing, branches, registry,
//! evals, MCP, config) plus a live event stream.

#![forbid(unsafe_code)]

pub mod diskio;
pub mod dto;
pub mod error;
pub mod routes;
pub mod spa;
pub mod sse;

use std::net::SocketAddr;

use atomr_agents_host::events::EventLog;
use atomr_agents_host::layout::HostPaths;
use atomr_agents_host::HostRuntime;
use axum::Router;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

#[derive(Clone, Debug)]
pub struct WebConfig {
    pub bind: SocketAddr,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            // 7000 stt, 7100 meetings, 7200 deep-research, 7300 coding-cli → 7400.
            bind: "127.0.0.1:7400".parse().expect("valid default addr"),
        }
    }
}

/// Shared application state handed to every route.
#[derive(Clone)]
pub struct AppState {
    pub runtime: HostRuntime,
    pub paths: HostPaths,
    pub events: EventLog,
}

impl AppState {
    pub fn new(runtime: HostRuntime) -> Self {
        let paths = runtime.config().paths.clone();
        let events = EventLog::new(paths.events_jsonl());
        Self {
            runtime,
            paths,
            events,
        }
    }
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
    pub fn new(config: WebConfig, runtime: HostRuntime) -> Self {
        Self {
            config,
            state: AppState::new(runtime),
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
