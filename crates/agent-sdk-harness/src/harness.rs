//! The harness — wraps an [`AgentSdkBackend`] behind a single async surface
//! plus a [`Callable`] impl.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::broadcast;
use tracing::info;

use atomr_agents_agent::SpendLedger;
use atomr_agents_agent_sdk_core::{
    AgentSdkBackend, AgentSdkConfig, AgentSdkEvent, AgentSdkEventStream, AgentRunId, MockBackend,
    QueryRequest, ResultSummary, SessionSpec,
};
use atomr_agents_callable::Callable;
use atomr_agents_core::{CallCtx, HarnessId, Result as CoreResult, Value};
use atomr_agents_observability::EventBus;

use crate::error::{HarnessError, Result};
use crate::headless;
use crate::projection::Projection;
use crate::session::{InteractiveAgentSession, SessionRegistry};
use crate::spec::AgentSdkHarnessSpec;

#[cfg(feature = "sandbox")]
use atomr_agents_sandbox_core::{CreateSandbox, SandboxId, SandboxInfo};
#[cfg(feature = "sandbox")]
use atomr_agents_sandbox_harness::SandboxHarness;
#[cfg(feature = "sandbox")]
use crate::workspace::{SessionWorkspace, WorkspaceDisposition, WorkspaceRegistry};

pub struct AgentSdkHarness {
    backend: Arc<dyn AgentSdkBackend>,
    pub spec: AgentSdkHarnessSpec,
    pub projection: Projection,
    pub bus: EventBus,
    event_tx: broadcast::Sender<AgentSdkEvent>,
    sessions: SessionRegistry,
    ledger: SpendLedger,
    harness_id: HarnessId,
    live_sessions: AtomicUsize,
    /// Sandbox orchestrator backing per-session workspaces (Pattern C).
    #[cfg(feature = "sandbox")]
    sandbox: Option<Arc<SandboxHarness>>,
    /// Live per-session workspaces, keyed by session id.
    #[cfg(feature = "sandbox")]
    workspaces: WorkspaceRegistry,
}

impl AgentSdkHarness {
    pub fn new(backend: Arc<dyn AgentSdkBackend>, spec: AgentSdkHarnessSpec) -> Self {
        let (event_tx, _) = broadcast::channel(spec.event_channel_capacity);
        let harness_id = HarnessId::from(spec.id.as_str());
        Self {
            backend,
            spec,
            projection: Projection::default(),
            bus: EventBus::new(),
            event_tx,
            sessions: SessionRegistry::new(),
            ledger: SpendLedger::new(),
            harness_id,
            live_sessions: AtomicUsize::new(0),
            #[cfg(feature = "sandbox")]
            sandbox: None,
            #[cfg(feature = "sandbox")]
            workspaces: WorkspaceRegistry::new(),
        }
    }

    /// Shortcut: the in-memory [`MockBackend`] + default spec. Network-free.
    pub fn local_default() -> Self {
        Self::new(Arc::new(MockBackend::new()), AgentSdkHarnessSpec::default())
    }

    /// Attach a `.claude/` projection materialized before each run.
    pub fn with_projection(mut self, projection: Projection) -> Self {
        self.projection = projection;
        self
    }

    /// Attach a [`SandboxHarness`] so sessions get a per-session isolated
    /// workspace when `spec.workspace.enabled` (Pattern C). The same
    /// `Arc<SandboxHarness>` must back the Python `run_in_sandbox` tool so both
    /// resolve the workspace sandbox from one registry.
    #[cfg(feature = "sandbox")]
    pub fn with_sandbox(mut self, sandbox: Arc<SandboxHarness>) -> Self {
        self.sandbox = Some(sandbox);
        self
    }

    pub fn events(&self) -> AgentSdkEventStream {
        AgentSdkEventStream::new(self.event_tx.subscribe())
    }

    pub fn event_sender(&self) -> broadcast::Sender<AgentSdkEvent> {
        self.event_tx.clone()
    }

    pub fn sessions(&self) -> &SessionRegistry {
        &self.sessions
    }

    pub fn ledger(&self) -> &SpendLedger {
        &self.ledger
    }

    pub fn backend_name(&self) -> &str {
        self.backend.name()
    }

    pub fn live_count(&self) -> usize {
        self.live_sessions.load(Ordering::SeqCst)
    }

    /// Apply spec defaults to a config that didn't override them.
    fn apply_defaults(&self, config: &mut AgentSdkConfig) {
        if config.model.is_none() {
            config.model = self.spec.default_model.clone();
        }
        if config.fallback_model.is_none() {
            config.fallback_model = self.spec.fallback_model.clone();
        }
        if config.max_turns.is_none() {
            config.max_turns = self.spec.default_max_turns;
        }
        if config.effort.is_none() {
            config.effort = self.spec.default_effort;
        }
        if config.setting_sources.is_empty() {
            config.setting_sources = self.spec.default_setting_sources.clone();
        }
    }

    fn validate_cwd(config: &AgentSdkConfig) -> Result<()> {
        if let Some(cwd) = &config.cwd {
            if !cwd.is_dir() {
                return Err(HarnessError::InvalidWorkdir(cwd.display().to_string()));
            }
        }
        Ok(())
    }

    fn materialize(&self, config: &AgentSdkConfig) -> Result<()> {
        if let Some(cwd) = &config.cwd {
            crate::projection::materialize(cwd.as_path() as &Path, &self.projection)?;
        }
        Ok(())
    }

    // ---- per-session sandbox workspaces (Pattern C) ----------------------

    /// Provision a per-session sandbox workspace and stage the `.claude/`
    /// projection into it. Returns `None` when the workspace feature is
    /// disabled in the spec (callers then fall back to host-fs materialization).
    /// Warm-forks from the snapshot pool when available, else cold-creates.
    #[cfg(feature = "sandbox")]
    async fn acquire_workspace(&self) -> Result<Option<SandboxInfo>> {
        let cfg = &self.spec.workspace;
        if !cfg.enabled {
            return Ok(None);
        }
        let sandbox = self
            .sandbox
            .as_ref()
            .ok_or(HarnessError::SandboxUnconfigured)?;
        let backend = cfg.backend.clone().unwrap_or_default();

        let mut req = CreateSandbox::new(cfg.profile).with_backend(backend);
        if cfg.reuse_warm {
            if let Some(snap) = sandbox.snapshot_pool().take(cfg.profile) {
                req.from_snapshot = Some(snap);
            }
        }
        let info = sandbox
            .create(req)
            .await
            .map_err(|e| HarnessError::Sandbox(e.to_string()))?;

        let handle = sandbox.get(&info.id).ok_or_else(|| {
            HarnessError::Sandbox("workspace sandbox vanished after create".into())
        })?;
        crate::projection::stage_into_sandbox(handle.as_ref(), &self.projection)
            .await
            .map_err(|e| HarnessError::Sandbox(e.to_string()))?;
        Ok(Some(info))
    }

    /// Acquire-or-materialize: when a sandbox workspace is provisioned, route
    /// the agent's exec/file into it (containment) and return the workspace to
    /// track; otherwise materialize the `.claude/` projection onto the host fs.
    #[cfg(feature = "sandbox")]
    async fn prepare_session_workspace(
        &self,
        config: &mut AgentSdkConfig,
    ) -> Result<Option<SessionWorkspace>> {
        match self.acquire_workspace().await? {
            Some(info) => {
                Self::apply_containment(config, &info.id);
                Ok(Some(SessionWorkspace {
                    sandbox_id: info.id,
                    profile: info.profile,
                    on_close: self.spec.workspace.on_close,
                }))
            }
            None => {
                self.materialize(config)?;
                Ok(None)
            }
        }
    }

    /// Tear down a per-session workspace per its disposition. Best-effort —
    /// teardown errors never block the caller (e.g. slot release).
    #[cfg(feature = "sandbox")]
    async fn release_workspace(&self, ws: SessionWorkspace) {
        let Some(sandbox) = self.sandbox.as_ref() else {
            return;
        };
        if ws.on_close == WorkspaceDisposition::Snapshot {
            if let Ok(snap) = sandbox.snapshot(&ws.sandbox_id).await {
                let _ = sandbox.snapshot_pool().offer(ws.profile, snap);
            }
        }
        let _ = sandbox.destroy(&ws.sandbox_id).await;
    }

    /// Route the agent into the session sandbox: pin the workspace id, remove
    /// the host built-in `Bash`/`Write`/`Edit`, and allow the in-process
    /// `run_in_sandbox` tool. Idempotent; respects user-supplied lists.
    #[cfg(feature = "sandbox")]
    fn apply_containment(config: &mut AgentSdkConfig, sandbox_id: &SandboxId) {
        config.sandbox_workspace_id = Some(sandbox_id.to_string());
        for t in ["Bash", "Write", "Edit"] {
            if !config.disallowed_tools.iter().any(|x| x == t) {
                config.disallowed_tools.push(t.to_string());
            }
        }
        let allow = "mcp__atomr__run_in_sandbox".to_string();
        if !config.allowed_tools.contains(&allow) {
            config.allowed_tools.push(allow);
        }
    }

    fn reserve_slot(&self) -> Result<()> {
        let prev = self.live_sessions.fetch_add(1, Ordering::SeqCst);
        if prev >= self.spec.max_concurrent_sessions {
            self.live_sessions.fetch_sub(1, Ordering::SeqCst);
            return Err(HarnessError::SessionQuota(self.spec.max_concurrent_sessions));
        }
        Ok(())
    }

    fn release_slot(&self) {
        self.live_sessions.fetch_sub(1, Ordering::SeqCst);
    }

    /// Drive one query to completion (headless).
    pub async fn run(&self, mut req: QueryRequest) -> Result<ResultSummary> {
        Self::validate_cwd(&req.config)?;
        self.apply_defaults(&mut req.config);
        if req.max_cost_usd.is_none() {
            req.max_cost_usd = self.spec.default_max_cost_usd;
        }

        // A headless run gets a throwaway workspace (always discarded), or
        // host-fs materialization when the sandbox feature is off/disabled.
        #[cfg(feature = "sandbox")]
        let workspace = self.prepare_session_workspace(&mut req.config).await?;
        #[cfg(not(feature = "sandbox"))]
        self.materialize(&req.config)?;

        let run_id = AgentRunId::new();
        info!(run_id = %run_id, model = ?req.config.model, "agent-sdk run starting");
        let _ = self.event_tx.send(AgentSdkEvent::RunStarted {
            run_id,
            model: req.config.model.clone(),
            session_id: None,
        });

        let max_cost = req.max_cost_usd;
        let outcome = match self.backend.query(req).await {
            Ok(stream) => {
                headless::drive(
                    stream,
                    &self.harness_id,
                    &self.event_tx,
                    &self.bus,
                    &self.ledger,
                    max_cost,
                )
                .await
            }
            Err(e) => Err(e.into()),
        };

        #[cfg(feature = "sandbox")]
        if let Some(mut ws) = workspace {
            ws.on_close = WorkspaceDisposition::Discard;
            self.release_workspace(ws).await;
        }

        outcome
    }

    /// Open a stateful interactive session.
    pub async fn start_session(&self, mut spec: SessionSpec) -> Result<Arc<InteractiveAgentSession>> {
        Self::validate_cwd(&spec.config)?;
        self.apply_defaults(&mut spec.config);

        self.reserve_slot()?;

        // Provision the per-session sandbox workspace (or host-fs projection).
        // Unwind the reserved agent slot if provisioning fails.
        #[cfg(feature = "sandbox")]
        let workspace = match self.prepare_session_workspace(&mut spec.config).await {
            Ok(w) => w,
            Err(e) => {
                self.release_slot();
                return Err(e);
            }
        };
        #[cfg(not(feature = "sandbox"))]
        if let Err(e) = self.materialize(&spec.config) {
            self.release_slot();
            return Err(e);
        }

        let initial = spec.initial_prompt.clone();
        let backend_session = match self.backend.create_session(spec).await {
            Ok(s) => s,
            Err(e) => {
                #[cfg(feature = "sandbox")]
                if let Some(ws) = workspace {
                    self.release_workspace(ws).await;
                }
                self.release_slot();
                return Err(e.into());
            }
        };
        let session = Arc::new(InteractiveAgentSession::new(
            backend_session,
            self.event_tx.clone(),
            self.bus.clone(),
            self.ledger.clone(),
            self.harness_id.clone(),
        ));
        #[cfg(feature = "sandbox")]
        if let Some(ws) = workspace {
            self.workspaces.insert(session.id.to_string(), ws);
        }
        self.sessions.insert(session.clone());
        if let Some(prompt) = initial {
            session.send(prompt).await?;
        }
        Ok(session)
    }

    /// Close an interactive session and drop it from the registry. Tears down
    /// its sandbox workspace (discard or snapshot) when one is attached.
    pub async fn stop_session(&self, id: &atomr_agents_agent_sdk_core::AgentSessionId) -> Result<()> {
        let s = self
            .sessions
            .get(id)
            .ok_or_else(|| HarnessError::SessionNotFound(id.to_string()))?;
        let _ = s.close().await;
        self.sessions.remove(id);
        #[cfg(feature = "sandbox")]
        if let Some(ws) = self.workspaces.take(&id.to_string()) {
            self.release_workspace(ws).await;
        }
        self.release_slot();
        Ok(())
    }
}

#[async_trait]
impl Callable for AgentSdkHarness {
    async fn call(&self, input: Value, _ctx: CallCtx) -> CoreResult<Value> {
        let req: QueryRequest = serde_json::from_value(input)?;
        let result = self.run(req).await.map_err(atomr_agents_core::AgentError::from)?;
        Ok(serde_json::to_value(result)?)
    }

    fn label(&self) -> &str {
        "agent-sdk-harness"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_agent_sdk_core::MockBackend;

    fn ctx() -> CallCtx {
        use atomr_agents_core::{IterationBudget, MoneyBudget, TimeBudget, TokenBudget};
        use std::time::Duration;
        CallCtx {
            agent_id: None,
            tokens: TokenBudget::new(1000),
            time: TimeBudget::new(Duration::from_secs(10)),
            money: MoneyBudget::from_usd(1.0),
            iterations: IterationBudget::new(10),
            trace: vec![],
            extensions: Default::default(),
        }
    }

    #[tokio::test]
    async fn run_returns_result_summary() {
        let h = AgentSdkHarness::new(
            Arc::new(MockBackend::new().with_reply("hi there")),
            AgentSdkHarnessSpec::default(),
        );
        let r = h.run(QueryRequest::new("hello")).await.unwrap();
        assert_eq!(r.subtype, "success");
        assert_eq!(r.result.as_deref(), Some("hi there"));
        // Ledger recorded a (zero-cost) spend entry.
        assert_eq!(h.ledger().total_tokens(), 2);
    }

    #[tokio::test]
    async fn callable_round_trips() {
        let h = AgentSdkHarness::local_default();
        let out = h
            .call(serde_json::json!({"prompt": "ping"}), ctx())
            .await
            .unwrap();
        assert_eq!(out["subtype"], "success");
    }

    #[tokio::test]
    async fn session_lifecycle() {
        let h = AgentSdkHarness::new(
            Arc::new(MockBackend::new().with_reply("ok")),
            AgentSdkHarnessSpec::default(),
        );
        let sess = h.start_session(SessionSpec::default()).await.unwrap();
        assert_eq!(h.live_count(), 1);
        let id = sess.id.clone();
        h.stop_session(&id).await.unwrap();
        assert_eq!(h.live_count(), 0);
    }

    #[tokio::test]
    async fn missing_cwd_rejected() {
        let h = AgentSdkHarness::local_default();
        let mut req = QueryRequest::new("x");
        req.config.cwd = Some("/no/such/dir/atomr-test".into());
        let err = h.run(req).await.unwrap_err();
        assert!(matches!(err, HarnessError::InvalidWorkdir(_)));
    }
}

#[cfg(all(test, feature = "sandbox"))]
mod sandbox_tests {
    use super::*;
    use async_trait::async_trait;
    use parking_lot::Mutex;

    use atomr_agents_agent_sdk_core::{AgentSdkError, AgentSdkSession, MessageStream};
    use atomr_agents_sandbox_core::{
        ExecRequest, Language, MockBackend as SbxMock, SandboxProfile, SnapshotId,
    };
    use atomr_agents_sandbox_harness::{BestFitScheduler, SandboxHarnessConfig};

    use crate::projection::{render_projection, Projection, SkillDoc};
    use crate::workspace::{SandboxWorkspaceConfig, WorkspaceDisposition};

    const PROFILE: SandboxProfile = SandboxProfile::PythonOnly;

    fn sandbox_spec() -> AgentSdkHarnessSpec {
        AgentSdkHarnessSpec {
            workspace: SandboxWorkspaceConfig {
                enabled: true,
                profile: PROFILE,
                backend: None,
                on_close: WorkspaceDisposition::Discard,
                reuse_warm: true,
            },
            ..Default::default()
        }
    }

    fn sample_projection() -> Projection {
        Projection {
            skills: vec![SkillDoc {
                id: "review".into(),
                name: Some("Review".into()),
                description: None,
                allowed_tools: vec![],
                body: "Do a review.".into(),
            }],
            ..Default::default()
        }
    }

    fn sandbox() -> Arc<SandboxHarness> {
        Arc::new(SandboxHarness::local_default())
    }

    /// Backend test double: records the config it receives, then delegates to
    /// `MockBackend` (which `MockBackend` alone can't do — it doesn't capture).
    struct CapturingBackend {
        inner: MockBackend,
        captured: Arc<Mutex<Option<AgentSdkConfig>>>,
    }

    #[async_trait]
    impl AgentSdkBackend for CapturingBackend {
        fn name(&self) -> &str {
            "capturing"
        }
        async fn available(&self) -> bool {
            true
        }
        async fn query(&self, req: QueryRequest) -> Result<MessageStream, AgentSdkError> {
            *self.captured.lock() = Some(req.config.clone());
            self.inner.query(req).await
        }
        async fn create_session(
            &self,
            spec: SessionSpec,
        ) -> Result<Box<dyn AgentSdkSession>, AgentSdkError> {
            *self.captured.lock() = Some(spec.config.clone());
            self.inner.create_session(spec).await
        }
    }

    #[tokio::test]
    async fn acquires_and_stages_projection() {
        let sb = sandbox();
        let projection = sample_projection();
        let h = AgentSdkHarness::new(Arc::new(MockBackend::new().with_reply("ok")), sandbox_spec())
            .with_projection(projection.clone())
            .with_sandbox(sb.clone());
        let _sess = h.start_session(SessionSpec::default()).await.unwrap();
        assert_eq!(sb.live_count(), 1);

        let infos = sb.list();
        assert_eq!(infos.len(), 1);
        let handle = sb.get(&infos[0].id).unwrap();
        let got = handle.read_file(".claude/skills/review/SKILL.md").await.unwrap();
        let expected = render_projection(&projection)
            .into_iter()
            .find(|(p, _)| p == ".claude/skills/review/SKILL.md")
            .unwrap()
            .1;
        assert_eq!(got, expected);
    }

    #[tokio::test]
    async fn exec_routes_to_session_sandbox() {
        let sb = sandbox();
        let h = AgentSdkHarness::new(Arc::new(MockBackend::new().with_reply("ok")), sandbox_spec())
            .with_sandbox(sb.clone());
        let _sess = h.start_session(SessionSpec::default()).await.unwrap();
        let id = sb.list()[0].id.clone();
        let res = sb
            .exec(&id, ExecRequest::new(Language::Python, "print(1)"))
            .await
            .unwrap();
        assert!(res.stdout.contains("[mock:Python]"));
    }

    #[tokio::test]
    async fn applies_containment_to_config() {
        let sb = sandbox();
        let captured = Arc::new(Mutex::new(None));
        let backend = Arc::new(CapturingBackend {
            inner: MockBackend::new().with_reply("ok"),
            captured: captured.clone(),
        });
        let h = AgentSdkHarness::new(backend, sandbox_spec()).with_sandbox(sb.clone());
        let _sess = h.start_session(SessionSpec::default()).await.unwrap();

        let cfg = captured.lock().clone().unwrap();
        assert!(cfg.sandbox_workspace_id.is_some());
        for t in ["Bash", "Write", "Edit"] {
            assert!(cfg.disallowed_tools.iter().any(|x| x == t), "missing {t}");
        }
        assert!(cfg.allowed_tools.iter().any(|x| x == "mcp__atomr__run_in_sandbox"));
    }

    #[tokio::test]
    async fn discard_on_close() {
        let sb = sandbox();
        let h = AgentSdkHarness::new(Arc::new(MockBackend::new().with_reply("ok")), sandbox_spec())
            .with_sandbox(sb.clone());
        let sess = h.start_session(SessionSpec::default()).await.unwrap();
        assert_eq!(sb.live_count(), 1);
        let id = sess.id.clone();
        h.stop_session(&id).await.unwrap();
        assert_eq!(sb.live_count(), 0);
        assert_eq!(h.live_count(), 0);
        assert!(h.workspaces.is_empty());
    }

    #[tokio::test]
    async fn snapshot_on_close() {
        let sb = sandbox();
        let mut spec = sandbox_spec();
        spec.workspace.on_close = WorkspaceDisposition::Snapshot;
        let h = AgentSdkHarness::new(Arc::new(MockBackend::new().with_reply("ok")), spec)
            .with_sandbox(sb.clone());
        assert_eq!(sb.snapshot_pool().len(PROFILE), 0);
        let sess = h.start_session(SessionSpec::default()).await.unwrap();
        let id = sess.id.clone();
        h.stop_session(&id).await.unwrap();
        assert_eq!(sb.snapshot_pool().len(PROFILE), 1);
        assert_eq!(sb.live_count(), 0);
    }

    #[tokio::test]
    async fn warm_reuse_from_pool() {
        let sb = sandbox();
        sb.snapshot_pool().offer(PROFILE, SnapshotId::new());
        let h = AgentSdkHarness::new(Arc::new(MockBackend::new().with_reply("ok")), sandbox_spec())
            .with_sandbox(sb.clone());
        let _sess = h.start_session(SessionSpec::default()).await.unwrap();
        let info = &sb.list()[0];
        assert!(info.forked_from.is_some());
        assert_eq!(sb.snapshot_pool().len(PROFILE), 0);
    }

    #[tokio::test]
    async fn disabled_uses_host_fs() {
        let sb = sandbox();
        let dir = tempfile::tempdir().unwrap();
        let h = AgentSdkHarness::new(
            Arc::new(MockBackend::new().with_reply("ok")),
            AgentSdkHarnessSpec::default(),
        )
        .with_projection(sample_projection())
        .with_sandbox(sb.clone());
        let mut spec = SessionSpec::default();
        spec.config.cwd = Some(dir.path().to_path_buf());
        let _sess = h.start_session(spec).await.unwrap();
        assert_eq!(sb.live_count(), 0);
        assert!(dir.path().join(".claude/skills/review/SKILL.md").is_file());
    }

    #[tokio::test]
    async fn enabled_without_client_errors() {
        let h = AgentSdkHarness::new(Arc::new(MockBackend::new().with_reply("ok")), sandbox_spec());
        let res = h.start_session(SessionSpec::default()).await;
        assert!(matches!(res, Err(HarnessError::SandboxUnconfigured)));
        assert_eq!(h.live_count(), 0);
    }

    #[tokio::test]
    async fn sandbox_quota_independent_of_session_quota() {
        let cfg = SandboxHarnessConfig { max_concurrent_sandboxes: 1, ..Default::default() };
        let sb = Arc::new(SandboxHarness::new(
            Arc::new(SbxMock::new()),
            Arc::new(BestFitScheduler),
            cfg,
        ));
        let mut spec = sandbox_spec();
        spec.max_concurrent_sessions = 2;
        let h = AgentSdkHarness::new(Arc::new(MockBackend::new().with_reply("ok")), spec)
            .with_sandbox(sb.clone());
        let _s1 = h.start_session(SessionSpec::default()).await.unwrap();
        assert_eq!(h.live_count(), 1);
        // 2nd session: sandbox quota (1) exhausted → error, agent slot unwound.
        let res = h.start_session(SessionSpec::default()).await;
        assert!(matches!(res, Err(HarnessError::Sandbox(_))));
        assert_eq!(h.live_count(), 1);
    }

    #[tokio::test]
    async fn headless_run_discards_workspace() {
        let sb = sandbox();
        let h = AgentSdkHarness::new(Arc::new(MockBackend::new().with_reply("ok")), sandbox_spec())
            .with_sandbox(sb.clone());
        let r = h.run(QueryRequest::new("hi")).await.unwrap();
        assert_eq!(r.subtype, "success");
        assert_eq!(sb.live_count(), 0);
    }
}
