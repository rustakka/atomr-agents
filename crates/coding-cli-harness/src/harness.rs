//! The harness — composes a `VendorRegistry`, an `Isolator`, and a
//! `CliRunStore` behind a single async surface plus a `Callable` impl.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::broadcast;
use tracing::info;

use atomr_agents_callable::Callable;
use atomr_agents_coding_cli_core::{
    CliRequest, CliResult, CliRunId, CliSessionId, CliVendorKind, CodingCliEvent,
    CodingCliEventStream, RunMode,
};
use atomr_agents_coding_cli_isolator::Isolator;
use atomr_agents_core::{CallCtx, Result as CoreResult, Value};
use atomr_agents_observability::EventBus;

use crate::dispatch::{encode_result, parse_request};
use crate::error::{HarnessError, Result};
use crate::headless;
use crate::interactive;
use crate::registry::VendorRegistry;
use crate::session::{InteractiveSessionHandle, SessionRegistry};
use crate::spec::CodingCliHarnessSpec;
use crate::store::{CliRunStore, InMemoryRunStore};

pub struct CodingCliHarness {
    pub spec: CodingCliHarnessSpec,
    pub vendors: VendorRegistry,
    pub isolator: Arc<dyn Isolator>,
    pub store: Arc<dyn CliRunStore>,
    pub bus: EventBus,
    pub(crate) event_tx: broadcast::Sender<CodingCliEvent>,
    pub(crate) sessions: SessionRegistry,
    cancel: Arc<AtomicBool>,
}

impl CodingCliHarness {
    pub fn new(
        spec: CodingCliHarnessSpec,
        vendors: VendorRegistry,
        isolator: Arc<dyn Isolator>,
        store: Arc<dyn CliRunStore>,
    ) -> Self {
        let (event_tx, _) = broadcast::channel(spec.event_channel_capacity);
        Self {
            spec,
            vendors,
            isolator,
            store,
            bus: EventBus::new(),
            event_tx,
            sessions: SessionRegistry::new(),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Shortcut: in-memory store + default vendors + local isolator.
    pub fn local_default() -> Self {
        use atomr_agents_coding_cli_isolator::LocalIsolator;
        Self::new(
            CodingCliHarnessSpec::default(),
            VendorRegistry::default_vendors(),
            Arc::new(LocalIsolator::new()),
            Arc::new(InMemoryRunStore::new()),
        )
    }

    /// Subscribe to the normalized event stream.
    pub fn events(&self) -> CodingCliEventStream {
        CodingCliEventStream::new(self.event_tx.subscribe())
    }

    /// Clone the broadcast sender — the web companion does this so its
    /// SSE handler can subscribe.
    pub fn event_sender(&self) -> broadcast::Sender<CodingCliEvent> {
        self.event_tx.clone()
    }

    /// List vendor kinds wired into this harness.
    pub fn available_vendors(&self) -> Vec<CliVendorKind> {
        self.vendors.kinds().cloned().collect()
    }

    pub fn sessions(&self) -> &SessionRegistry {
        &self.sessions
    }

    /// Cancel any in-flight headless run (cooperative).
    pub fn cancel(&self) {
        self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Drive one request to completion.
    pub async fn run(&self, mut req: CliRequest) -> Result<CliResult> {
        validate_workdir(&req)?;
        if req.mode != RunMode::Headless {
            return Err(HarnessError::InvalidRequest(
                "run() drives headless mode only; use start_interactive() for interactive runs".into(),
            ));
        }
        if req.model.is_none() {
            req.model = self.spec.default_model.clone();
        }
        let vendor = self
            .vendors
            .get(&req.vendor)
            .ok_or_else(|| HarnessError::UnknownVendor(req.vendor.clone()))?;

        let run_id = CliRunId::new();
        info!(run_id = %run_id, vendor = %req.vendor, "headless run starting");

        // Persist a placeholder result so the store has an entry as
        // soon as the run is known (UI can poll).
        let mut placeholder = CliResult::new(run_id.clone(), req.vendor.clone());
        placeholder.started_at = chrono::Utc::now();
        self.store.put(&placeholder).await?;

        let cancel = self.cancel.clone();
        let event_tx = self.event_tx.clone();

        let result = headless::run_one(run_id.clone(), vendor, self.isolator.clone(), req, event_tx, cancel).await?;
        self.store.put(&result).await?;
        Ok(result)
    }

    /// Spawn an interactive session and register it. Returns the new
    /// `CliSessionId`; clients should connect to
    /// `WS /api/cli/sessions/{id}/io` (in the web companion) to drive it.
    pub async fn start_interactive(&self, mut req: CliRequest) -> Result<Arc<InteractiveSessionHandle>> {
        validate_workdir(&req)?;
        req.mode = RunMode::Interactive;
        if req.model.is_none() {
            req.model = self.spec.default_model.clone();
        }
        if self.sessions.len() >= self.spec.max_concurrent_sessions {
            return Err(HarnessError::InvalidRequest(format!(
                "max_concurrent_sessions reached ({})",
                self.spec.max_concurrent_sessions
            )));
        }
        let vendor = self
            .vendors
            .get(&req.vendor)
            .ok_or_else(|| HarnessError::UnknownVendor(req.vendor.clone()))?;
        let id = CliSessionId::new();
        let handle = interactive::start_session(id, vendor, self.isolator.clone(), req).await?;
        self.sessions.insert(handle.clone());
        Ok(handle)
    }

    /// Stop an interactive session and remove it from the registry.
    pub async fn stop_interactive(&self, id: &CliSessionId) -> Result<()> {
        let h = self
            .sessions
            .get(id)
            .ok_or_else(|| HarnessError::SessionNotFound(id.to_string()))?;
        let _ = h.detach().await;
        interactive::stop_session(self.isolator.clone(), &h.tmux_session, h.request.workdir.clone()).await?;
        self.sessions.remove(id);
        Ok(())
    }
}

fn validate_workdir(req: &CliRequest) -> Result<()> {
    if !req.workdir.is_dir() {
        return Err(HarnessError::InvalidWorkdir(req.workdir.display().to_string()));
    }
    Ok(())
}

#[async_trait]
impl Callable for CodingCliHarness {
    async fn call(&self, input: Value, _ctx: CallCtx) -> CoreResult<Value> {
        let req = parse_request(input)?;
        let result = self.run(req).await.map_err(atomr_agents_core::AgentError::from)?;
        encode_result(&result)
    }

    fn label(&self) -> &str {
        "coding-cli-harness"
    }
}
