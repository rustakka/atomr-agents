//! `SandboxHarness` — the orchestrator. Owns the injected backend, a registry
//! of live sandboxes, a scheduler + snapshot pool, an `EventBus`, and a
//! broadcast channel of [`SandboxEvent`]s. Mirrors `CodingCliHarness`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use atomr_agents_callable::Callable;
use atomr_agents_core::{AgentError, CallCtx, Result as CoreResult, Value};
use atomr_agents_observability::EventBus;
use atomr_agents_sandbox_core::{
    CreateSandbox, ExecRequest, ExecResult, MockBackend, SandboxBackend, SandboxBackendSel,
    SandboxError, SandboxEvent, SandboxEventStream, SandboxHandle, SandboxId, SandboxInfo,
    SandboxProfile, SnapshotId,
};
use dashmap::DashMap;
use serde::Deserialize;
use tokio::sync::broadcast;

use crate::pool::SnapshotPool;
use crate::scheduler::{BestFitScheduler, Scheduler};

/// Tunables for the harness.
#[derive(Debug, Clone)]
pub struct SandboxHarnessConfig {
    pub event_channel_capacity: usize,
    /// Hard cap on simultaneously-live sandboxes in the registry (quota).
    pub max_concurrent_sandboxes: usize,
    /// Warm snapshots retained per profile.
    pub snapshot_pool_capacity: usize,
}

impl Default for SandboxHarnessConfig {
    fn default() -> Self {
        Self {
            event_channel_capacity: 256,
            max_concurrent_sandboxes: 64,
            snapshot_pool_capacity: 4,
        }
    }
}

/// Callable / convenience input: an exec plus optional profile + backend
/// selection. Shared shape with the `execute_in_sandbox` tool args.
#[derive(Debug, Deserialize)]
struct CallInput {
    #[serde(default)]
    profile: Option<SandboxProfile>,
    #[serde(default)]
    backend: SandboxBackendSel,
    #[serde(flatten)]
    exec: ExecRequest,
}

pub struct SandboxHarness {
    backend: Arc<dyn SandboxBackend>,
    pub bus: EventBus,
    event_tx: broadcast::Sender<SandboxEvent>,
    sandboxes: DashMap<SandboxId, Arc<dyn SandboxHandle>>,
    /// Admission counter for the concurrency quota. Reserved *before* the
    /// provisioning await so concurrent `create`/`fork` calls can't race past
    /// the cap (the registry `len()` check would be TOCTOU across the await).
    live: AtomicUsize,
    pool: Arc<SnapshotPool>,
    scheduler: Arc<dyn Scheduler>,
    config: SandboxHarnessConfig,
}

impl SandboxHarness {
    pub fn new(
        backend: Arc<dyn SandboxBackend>,
        scheduler: Arc<dyn Scheduler>,
        config: SandboxHarnessConfig,
    ) -> Self {
        let (event_tx, _) = broadcast::channel(config.event_channel_capacity);
        let pool = Arc::new(SnapshotPool::new(config.snapshot_pool_capacity));
        Self {
            backend,
            bus: EventBus::new(),
            event_tx,
            sandboxes: DashMap::new(),
            live: AtomicUsize::new(0),
            pool,
            scheduler,
            config,
        }
    }

    /// Shortcut: mock backend + best-fit scheduler + default config. The
    /// cross-platform default used by tests and the PyO3 bindings.
    pub fn local_default() -> Self {
        Self::new(
            Arc::new(MockBackend::new()),
            Arc::new(BestFitScheduler),
            SandboxHarnessConfig::default(),
        )
    }

    /// Subscribe to the lifecycle event stream.
    pub fn events(&self) -> SandboxEventStream {
        SandboxEventStream::new(self.event_tx.subscribe())
    }

    /// Clone the broadcast sender — the web companion subscribes through this.
    pub fn event_sender(&self) -> broadcast::Sender<SandboxEvent> {
        self.event_tx.clone()
    }

    pub fn backend_name(&self) -> &str {
        self.backend.name()
    }

    pub fn scheduler(&self) -> &Arc<dyn Scheduler> {
        &self.scheduler
    }

    pub fn snapshot_pool(&self) -> &Arc<SnapshotPool> {
        &self.pool
    }

    pub fn live_count(&self) -> usize {
        self.sandboxes.len()
    }

    fn broadcast(&self, ev: SandboxEvent) {
        // Best-effort: no subscribers is fine.
        let _ = self.event_tx.send(ev);
    }

    /// Atomically reserve a concurrency slot before provisioning. Reserving up
    /// front (rather than checking `len()` after the await) closes the TOCTOU
    /// window between admission and registration.
    fn reserve_slot(&self) -> Result<(), SandboxError> {
        if self.live.fetch_add(1, Ordering::SeqCst) >= self.config.max_concurrent_sandboxes {
            self.live.fetch_sub(1, Ordering::SeqCst);
            return Err(SandboxError::BudgetViolation(format!(
                "max concurrent sandboxes ({}) reached",
                self.config.max_concurrent_sandboxes
            )));
        }
        Ok(())
    }

    fn release_slot(&self) {
        self.live.fetch_sub(1, Ordering::SeqCst);
    }

    /// Reject an explicit backend selection the injected backend can't honor,
    /// rather than silently running on whatever is wired. `Auto` always
    /// accepts the injected backend.
    fn ensure_backend_available(&self, sel: &SandboxBackendSel) -> Result<(), SandboxError> {
        let name = self.backend.name();
        let ok = match sel {
            SandboxBackendSel::Auto => true,
            SandboxBackendSel::Mock => name == "mock",
            SandboxBackendSel::Docker { .. } => name == "docker",
            SandboxBackendSel::Firecracker => name == "firecracker",
            SandboxBackendSel::Remote { .. } => name == "remote",
        };
        if ok {
            Ok(())
        } else {
            Err(SandboxError::Unsupported(
                "requested sandbox backend is not available on this harness",
            ))
        }
    }

    /// Pin the provisioning budget to the effective (Rust-floored) value so the
    /// 2 GB / 2 vCPU floor is structural — a backend that reads `req.budget`
    /// directly can no longer undercut it.
    fn normalize(&self, mut req: CreateSandbox) -> CreateSandbox {
        req.budget = Some(req.effective_budget());
        req
    }

    // ---- ephemeral one-shot path (used by the tool) ----------------------

    /// Resolve a profile from the language when none is given, then run a
    /// single exec in a fresh ephemeral sandbox. The primary path for the
    /// `execute_in_sandbox` tool.
    pub async fn run_exec(
        &self,
        exec: ExecRequest,
        profile: Option<SandboxProfile>,
        backend: SandboxBackendSel,
    ) -> Result<ExecResult, SandboxError> {
        let profile = profile.unwrap_or_else(|| SandboxProfile::for_language(exec.language));
        let create = CreateSandbox::new(profile).with_backend(backend);
        self.run_once(create, exec).await
    }

    /// Create a sandbox, run one exec, destroy it. Emits the full lifecycle.
    pub async fn run_once(
        &self,
        create: CreateSandbox,
        exec: ExecRequest,
    ) -> Result<ExecResult, SandboxError> {
        self.ensure_backend_available(&create.backend)?;
        // Fail fast before booting if the profile can't run the language.
        create.profile.ensure_supports(exec.language)?;
        let create = self.normalize(create);
        let handle = self.backend.create(create).await?;
        let info = handle.info().clone();
        self.broadcast(SandboxEvent::Created {
            id: info.id.clone(),
            profile: info.profile,
            boot_ms: info.boot_ms,
        });
        self.broadcast(SandboxEvent::ExecStarted {
            id: info.id.clone(),
            language: exec.language,
        });
        let started = Instant::now();
        let result = handle.exec(exec).await;
        match &result {
            Ok(res) => self.broadcast(SandboxEvent::ExecEnded {
                id: info.id.clone(),
                exec_id: res.exec_id.clone(),
                exit_code: res.exit.code,
                elapsed_ms: started.elapsed().as_millis() as u64,
            }),
            Err(e) => self.broadcast(SandboxEvent::ExecError {
                id: info.id.clone(),
                error: e.to_string(),
            }),
        }
        let _ = handle.destroy().await;
        self.broadcast(SandboxEvent::Destroyed { id: info.id.clone() });
        result
    }

    // ---- persistent registry path (used by SandboxClient) ----------------

    /// Provision a persistent sandbox kept in the registry. Enforces the
    /// concurrency quota.
    pub async fn create(&self, req: CreateSandbox) -> Result<SandboxInfo, SandboxError> {
        self.ensure_backend_available(&req.backend)?;
        self.reserve_slot()?;
        let req = self.normalize(req);
        let handle: Arc<dyn SandboxHandle> = match self.backend.create(req).await {
            Ok(h) => Arc::from(h),
            Err(e) => {
                self.release_slot();
                return Err(e);
            }
        };
        let info = handle.info().clone();
        self.sandboxes.insert(info.id.clone(), handle);
        self.broadcast(SandboxEvent::Created {
            id: info.id.clone(),
            profile: info.profile,
            boot_ms: info.boot_ms,
        });
        Ok(info)
    }

    pub fn get(&self, id: &SandboxId) -> Option<Arc<dyn SandboxHandle>> {
        self.sandboxes.get(id).map(|e| e.value().clone())
    }

    pub fn list(&self) -> Vec<SandboxInfo> {
        self.sandboxes.iter().map(|e| e.value().info().clone()).collect()
    }

    /// Exec inside a registered sandbox, emitting lifecycle events.
    pub async fn exec(&self, id: &SandboxId, req: ExecRequest) -> Result<ExecResult, SandboxError> {
        let handle = self
            .get(id)
            .ok_or_else(|| SandboxError::NotFound(id.clone()))?;
        self.broadcast(SandboxEvent::ExecStarted {
            id: id.clone(),
            language: req.language,
        });
        let started = Instant::now();
        let result = handle.exec(req).await;
        match &result {
            Ok(res) => self.broadcast(SandboxEvent::ExecEnded {
                id: id.clone(),
                exec_id: res.exec_id.clone(),
                exit_code: res.exit.code,
                elapsed_ms: started.elapsed().as_millis() as u64,
            }),
            Err(e) => self.broadcast(SandboxEvent::ExecError {
                id: id.clone(),
                error: e.to_string(),
            }),
        }
        result
    }

    /// Snapshot a registered sandbox.
    pub async fn snapshot(&self, id: &SandboxId) -> Result<SnapshotId, SandboxError> {
        let handle = self
            .get(id)
            .ok_or_else(|| SandboxError::NotFound(id.clone()))?;
        handle.snapshot().await
    }

    /// Fork a registered sandbox into a new registered child (backs agent
    /// branch states).
    pub async fn fork(&self, id: &SandboxId) -> Result<SandboxInfo, SandboxError> {
        let parent = self
            .get(id)
            .ok_or_else(|| SandboxError::NotFound(id.clone()))?;
        // A fork registers a new sandbox, so it counts against the quota too.
        self.reserve_slot()?;
        let child: Arc<dyn SandboxHandle> = match parent.fork().await {
            Ok(c) => Arc::from(c),
            Err(e) => {
                self.release_slot();
                return Err(e);
            }
        };
        let info = child.info().clone();
        let snapshot = info.forked_from.clone().unwrap_or_default();
        self.sandboxes.insert(info.id.clone(), child);
        self.broadcast(SandboxEvent::Forked {
            parent: id.clone(),
            child: info.id.clone(),
            snapshot,
        });
        Ok(info)
    }

    /// Destroy and de-register a sandbox.
    pub async fn destroy(&self, id: &SandboxId) -> Result<(), SandboxError> {
        match self.sandboxes.remove(id) {
            Some((_, handle)) => {
                // Free the slot as soon as it leaves the registry, regardless
                // of whether teardown errors.
                self.release_slot();
                handle.destroy().await?;
                self.broadcast(SandboxEvent::Destroyed { id: id.clone() });
                Ok(())
            }
            None => Err(SandboxError::NotFound(id.clone())),
        }
    }
}

#[async_trait]
impl Callable for SandboxHarness {
    async fn call(&self, input: Value, _ctx: CallCtx) -> CoreResult<Value> {
        let parsed: CallInput = serde_json::from_value(input).map_err(AgentError::from)?;
        let res = self
            .run_exec(parsed.exec, parsed.profile, parsed.backend)
            .await
            .map_err(|e| AgentError::Tool(format!("sandbox: {e}")))?;
        Ok(res.to_tool_json())
    }

    fn label(&self) -> &str {
        "sandbox-harness"
    }
}
