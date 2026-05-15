//! The typed deep-research harness + shared run loop.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::Callable;
use atomr_agents_core::{CallCtx, Event, Result as CoreResult, RunId, Value};
use atomr_agents_deep_research_core::{ResearchRequest, ResearchResult, ResearchState};
use atomr_agents_observability::EventBus;
use atomr_agents_retriever::Retriever;
use atomr_agents_web_search_core::WebSearch;
use parking_lot::Mutex;
use tokio::sync::broadcast;

use crate::boxed::BoxedDeepResearchHarness;
use crate::dispatch::DeepResearchHarnessDispatch;
use crate::error::{DeepResearchError, Result};
use crate::events::{DeepResearchEvent, DeepResearchEventStream};
use crate::handle::ResearchHandle;
use crate::loop_strategy::{DeepResearchLoopStrategy, DeepResearchStepCtx, DeepResearchStepOutcome};
use crate::roles::{CitationVerifier, Clarifier, Critic, Planner, Researcher, Writer};
use crate::spec::DeepResearchHarnessSpec;
use crate::state::{DeepResearchState, DeepResearchStepEvent};
use crate::store::ResearchStore;
use crate::termination::{DeepResearchTermination, Termination};

const EVENT_CHANNEL_CAPACITY: usize = 512;

/// The full set of role implementations a harness instance is wired
/// with. Kept as one struct so callers can construct a harness with
/// `roles: DeepResearchRoles { ... }` rather than naming six arguments.
pub struct DeepResearchRoles {
    pub clarifier: Arc<dyn Clarifier>,
    pub planner: Arc<dyn Planner>,
    pub researcher: Arc<dyn Researcher>,
    pub writer: Arc<dyn Writer>,
    pub critic: Arc<dyn Critic>,
    pub verifier: Arc<dyn CitationVerifier>,
}

impl DeepResearchRoles {
    /// Default deterministic LLM-free roles (mirrors the meetings
    /// harness's `RuleBasedExtractor` pattern).
    pub fn defaults() -> Self {
        use crate::roles::{
            ConcatWriter, DeterministicCitationVerifier, HeuristicPlanner, MockResearcher, RegexCritic,
            TemplateClarifier,
        };
        Self {
            clarifier: Arc::new(TemplateClarifier::new()),
            planner: Arc::new(HeuristicPlanner::new()),
            researcher: Arc::new(MockResearcher::new()),
            writer: Arc::new(ConcatWriter::new()),
            critic: Arc::new(RegexCritic::new()),
            verifier: Arc::new(DeterministicCitationVerifier::new()),
        }
    }
}

/// A typed deep-research harness.
pub struct DeepResearchHarness<L, T>
where
    L: DeepResearchLoopStrategy,
    T: DeepResearchTermination,
{
    pub spec: DeepResearchHarnessSpec,
    pub store: Arc<dyn ResearchStore>,
    pub search: Arc<dyn WebSearch>,
    pub retriever: Option<Arc<dyn Retriever>>,
    pub roles: DeepResearchRoles,
    pub loop_strategy: L,
    pub termination: T,
    pub bus: EventBus,
    pub(crate) event_tx: broadcast::Sender<DeepResearchEvent>,
    cancel: Arc<parking_lot::Mutex<bool>>,
}

impl<L, T> DeepResearchHarness<L, T>
where
    L: DeepResearchLoopStrategy,
    T: DeepResearchTermination,
{
    pub fn new(
        spec: DeepResearchHarnessSpec,
        store: Arc<dyn ResearchStore>,
        search: Arc<dyn WebSearch>,
        roles: DeepResearchRoles,
        loop_strategy: L,
        termination: T,
    ) -> Self {
        let (event_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
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

    /// Run the harness against a [`ResearchRequest`].
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
            &self.loop_strategy as &dyn DeepResearchLoopStrategy,
            &self.termination as &dyn DeepResearchTermination,
            self.store.clone(),
            &handle,
            &self.event_tx,
            self.cancel.clone(),
            &self.bus,
            &self.roles,
            &mut state,
        )
        .await;

        // Persist final state (success OR failure).
        match final_reason {
            Ok(_) => {
                handle.finalize();
            }
            Err(e) => {
                handle.fail(e.to_string());
            }
        }
        let final_result = result_arc.lock().clone();
        self.store.put(&final_result).await?;
        Ok(final_result)
    }

    pub fn into_boxed(self) -> BoxedDeepResearchHarness {
        BoxedDeepResearchHarness {
            spec: self.spec,
            store: self.store,
            search: self.search,
            retriever: self.retriever,
            roles: self.roles,
            loop_strategy: Box::new(self.loop_strategy),
            termination: Box::new(self.termination),
            bus: self.bus,
            event_tx: self.event_tx,
            cancel: self.cancel,
        }
    }
}

#[async_trait]
impl<L, T> Callable for DeepResearchHarness<L, T>
where
    L: DeepResearchLoopStrategy,
    T: DeepResearchTermination,
{
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
impl<L, T> DeepResearchHarnessDispatch for DeepResearchHarness<L, T>
where
    L: DeepResearchLoopStrategy,
    T: DeepResearchTermination,
{
    async fn dispatch(&self, request: ResearchRequest) -> CoreResult<Value> {
        let result = self.run(request).await?;
        Ok(serde_json::to_value(result).map_err(DeepResearchError::from)?)
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_loop(
    spec: &DeepResearchHarnessSpec,
    run_id: &RunId,
    loop_strategy: &dyn DeepResearchLoopStrategy,
    termination: &dyn DeepResearchTermination,
    store: Arc<dyn ResearchStore>,
    handle: &ResearchHandle,
    events: &broadcast::Sender<DeepResearchEvent>,
    cancel: Arc<parking_lot::Mutex<bool>>,
    bus: &EventBus,
    roles: &DeepResearchRoles,
    state: &mut DeepResearchState,
) -> Result<&'static str> {
    let final_reason: &'static str = loop {
        if *cancel.lock() {
            state.cancel_requested = true;
        }
        if let Termination::Done(reason) = termination.should_terminate(state) {
            emit_iteration(
                bus,
                spec,
                run_id,
                state.iteration,
                &format!("terminated:{reason}"),
            );
            break reason;
        }
        state.iteration += 1;

        let mut ctx = DeepResearchStepCtx {
            state,
            handle,
            store: store.clone(),
            clarifier: roles.clarifier.as_ref(),
            planner: roles.planner.as_ref(),
            researcher: roles.researcher.as_ref(),
            writer: roles.writer.as_ref(),
            critic: roles.critic.as_ref(),
            verifier: roles.verifier.as_ref(),
            events,
        };
        let outcome = loop_strategy.step(&mut ctx).await?;
        let label_owned = match &outcome {
            DeepResearchStepOutcome::Continue { label } => label.clone(),
            DeepResearchStepOutcome::Done { label } => label.clone(),
        };
        state.history.push(DeepResearchStepEvent {
            iteration: state.iteration,
            outcome: label_owned.clone(),
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
        });
        emit_iteration(bus, spec, run_id, state.iteration, &label_owned);

        // Persist incrementally so the web UI sees in-flight progress.
        let snap = handle.snapshot();
        state.result = snap.clone();
        store.put(&snap).await?;

        if matches!(outcome, DeepResearchStepOutcome::Done { .. }) {
            break "complete";
        }
    };
    Ok(final_reason)
}

fn emit_iteration(
    bus: &EventBus,
    spec: &DeepResearchHarnessSpec,
    run_id: &RunId,
    iteration: u64,
    outcome: &str,
) {
    bus.emit_run(
        Event::HarnessIteration {
            harness_id: spec.id.clone(),
            iteration,
            outcome: outcome.to_string(),
            budget_remaining_tokens: 0,
        },
        run_id.clone(),
        None,
    );
}
