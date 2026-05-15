//! Type-erased deep-research harness.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::Callable;
use atomr_agents_core::{CallCtx, Result as CoreResult, RunId, Value};
use atomr_agents_deep_research_core::{ResearchRequest, ResearchResult, ResearchState};
use atomr_agents_observability::EventBus;
use atomr_agents_retriever::Retriever;
use atomr_agents_web_search_core::WebSearch;
use parking_lot::Mutex;
use tokio::sync::broadcast;

use crate::dispatch::DeepResearchHarnessDispatch;
use crate::error::{DeepResearchError, Result};
use crate::events::{DeepResearchEvent, DeepResearchEventStream};
use crate::handle::ResearchHandle;
use crate::harness::{run_loop, DeepResearchRoles};
use crate::loop_strategy::DeepResearchLoopStrategy;
use crate::spec::DeepResearchHarnessSpec;
use crate::state::DeepResearchState;
use crate::store::ResearchStore;
use crate::termination::DeepResearchTermination;

/// Type-erased deep-research harness.
pub struct BoxedDeepResearchHarness {
    pub spec: DeepResearchHarnessSpec,
    pub store: Arc<dyn ResearchStore>,
    pub search: Arc<dyn WebSearch>,
    pub retriever: Option<Arc<dyn Retriever>>,
    pub roles: DeepResearchRoles,
    pub loop_strategy: Box<dyn DeepResearchLoopStrategy>,
    pub termination: Box<dyn DeepResearchTermination>,
    pub bus: EventBus,
    pub(crate) event_tx: broadcast::Sender<DeepResearchEvent>,
    pub(crate) cancel: Arc<parking_lot::Mutex<bool>>,
}

impl BoxedDeepResearchHarness {
    pub fn new(
        spec: DeepResearchHarnessSpec,
        store: Arc<dyn ResearchStore>,
        search: Arc<dyn WebSearch>,
        roles: DeepResearchRoles,
        loop_strategy: Box<dyn DeepResearchLoopStrategy>,
        termination: Box<dyn DeepResearchTermination>,
    ) -> Self {
        let (event_tx, _) = broadcast::channel(512);
        Self {
            spec,
            store,
            search,
            retriever: None,
            roles,
            loop_strategy,
            termination,
            bus: EventBus::new(),
            event_tx,
            cancel: Arc::new(parking_lot::Mutex::new(false)),
        }
    }

    pub fn with_retriever(mut self, retriever: Arc<dyn Retriever>) -> Self {
        self.retriever = Some(retriever);
        self
    }

    pub fn events(&self) -> DeepResearchEventStream {
        DeepResearchEventStream::new(self.event_tx.subscribe())
    }

    pub fn event_sender(&self) -> broadcast::Sender<DeepResearchEvent> {
        self.event_tx.clone()
    }

    pub fn cancel(&self) {
        *self.cancel.lock() = true;
    }

    pub async fn run(&self, request: ResearchRequest) -> Result<ResearchResult> {
        let mut result = ResearchResult::new(request.query.clone(), self.loop_strategy.name());
        result.model_id = self.spec.model_id.clone();
        self.store.put(&result).await?;

        let result_arc = Arc::new(Mutex::new(result.clone()));
        let request_arc = Arc::new(request);
        let mut handle = ResearchHandle::new(result_arc.clone(), request_arc.clone(), self.search.clone());
        if let Some(r) = &self.retriever {
            handle = handle.with_retriever(r.clone());
        }
        let handle = handle.with_events(self.event_tx.clone());

        let _ = self.event_tx.send(DeepResearchEvent::Started {
            strategy: self.loop_strategy.name().to_string(),
            query: request_arc.query.clone(),
        });
        handle.set_state(ResearchState::Running);

        let mut state = DeepResearchState::new(result_arc.lock().clone(), self.spec.initial_budget.remaining);
        let run_id = RunId::new();
        let final_reason = run_loop(
            &self.spec,
            &run_id,
            &*self.loop_strategy,
            &*self.termination,
            self.store.clone(),
            &handle,
            &self.event_tx,
            self.cancel.clone(),
            &self.bus,
            &self.roles,
            &mut state,
        )
        .await;
        match final_reason {
            Ok(_) => handle.finalize(),
            Err(e) => handle.fail(e.to_string()),
        }
        let final_result = result_arc.lock().clone();
        self.store.put(&final_result).await?;
        Ok(final_result)
    }
}

#[async_trait]
impl Callable for BoxedDeepResearchHarness {
    async fn call(&self, input: Value, _ctx: CallCtx) -> CoreResult<Value> {
        let request = crate::dispatch::parse_request(input)?;
        let result = self.run(request).await?;
        Ok(serde_json::to_value(result).map_err(DeepResearchError::from)?)
    }
    fn label(&self) -> &str {
        self.spec.id.as_str()
    }
}

#[async_trait]
impl DeepResearchHarnessDispatch for BoxedDeepResearchHarness {
    async fn dispatch(&self, request: ResearchRequest) -> CoreResult<Value> {
        let result = self.run(request).await?;
        Ok(serde_json::to_value(result).map_err(DeepResearchError::from)?)
    }
}
