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
        self.materialize(&req.config)?;

        let run_id = AgentRunId::new();
        info!(run_id = %run_id, model = ?req.config.model, "agent-sdk run starting");
        let _ = self.event_tx.send(AgentSdkEvent::RunStarted {
            run_id,
            model: req.config.model.clone(),
            session_id: None,
        });

        let max_cost = req.max_cost_usd;
        let stream = self.backend.query(req).await?;
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

    /// Open a stateful interactive session.
    pub async fn start_session(&self, mut spec: SessionSpec) -> Result<Arc<InteractiveAgentSession>> {
        Self::validate_cwd(&spec.config)?;
        self.apply_defaults(&mut spec.config);
        self.materialize(&spec.config)?;

        self.reserve_slot()?;
        let initial = spec.initial_prompt.clone();
        let backend_session = match self.backend.create_session(spec).await {
            Ok(s) => s,
            Err(e) => {
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
        self.sessions.insert(session.clone());
        if let Some(prompt) = initial {
            session.send(prompt).await?;
        }
        Ok(session)
    }

    /// Close an interactive session and drop it from the registry.
    pub async fn stop_session(&self, id: &atomr_agents_agent_sdk_core::AgentSessionId) -> Result<()> {
        let s = self
            .sessions
            .get(id)
            .ok_or_else(|| HarnessError::SessionNotFound(id.to_string()))?;
        let _ = s.close().await;
        self.sessions.remove(id);
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
