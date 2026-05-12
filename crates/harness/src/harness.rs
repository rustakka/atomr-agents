use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::Callable;
use atomr_agents_core::{AgentError, CallCtx, Event, HarnessId, Result, TokenBudget, Value};
use atomr_agents_observability::EventBus;
use semver::Version;

use crate::boxed::BoxedHarness;
use crate::dispatch::{HarnessDispatch, HarnessRef};
use crate::loop_strategy::{LoopStrategy, StepOutcome};
use crate::state::{HarnessState, StepEvent};
use crate::termination::{Termination, TerminationStrategy};

#[derive(Clone)]
pub struct HarnessSpec {
    pub id: HarnessId,
    pub version: Version,
    pub eval_suite_id: Option<String>,
    pub initial_budget: TokenBudget,
}

impl HarnessSpec {
    /// Materialize a runnable `HarnessRef` from a static spec + concrete
    /// strategies. Mirrors `AgentSpec::into_agent` (W3a). Both strategies
    /// are passed as boxed trait objects so callers without access to the
    /// concrete generic types (Python config loaders, registry-driven
    /// dispatchers) can construct one.
    pub fn into_harness(
        self,
        loop_strategy: Box<dyn LoopStrategy>,
        termination: Box<dyn TerminationStrategy>,
    ) -> HarnessRef {
        let id = self.id.clone();
        let boxed = BoxedHarness {
            spec: self,
            loop_strategy,
            termination,
            bus: EventBus::new(),
        };
        HarnessRef::new(id, Arc::new(boxed))
    }
}

pub struct Harness<L, T>
where
    L: LoopStrategy,
    T: TerminationStrategy,
{
    pub spec: HarnessSpec,
    pub loop_strategy: L,
    pub termination: T,
    pub bus: EventBus,
}

impl<L, T> Harness<L, T>
where
    L: LoopStrategy,
    T: TerminationStrategy,
{
    pub async fn run(&self) -> Result<Value> {
        run_impl(&self.spec, &self.loop_strategy, &self.termination, &self.bus).await
    }

    /// Consume this typed harness and return its fully-erased boxed
    /// equivalent. The strategies are moved into `Box<dyn ...>` slots so
    /// the result is a uniform `BoxedHarness` regardless of the original
    /// generic parameters.
    pub fn into_boxed(self) -> BoxedHarness {
        BoxedHarness {
            spec: self.spec,
            loop_strategy: Box::new(self.loop_strategy),
            termination: Box::new(self.termination),
            bus: self.bus,
        }
    }
}

#[async_trait]
impl<L, T> Callable for Harness<L, T>
where
    L: LoopStrategy,
    T: TerminationStrategy,
{
    async fn call(&self, _input: Value, _ctx: CallCtx) -> Result<Value> {
        self.run().await
    }

    fn label(&self) -> &str {
        self.spec.id.as_str()
    }
}

#[async_trait]
impl<L, T> HarnessDispatch for Harness<L, T>
where
    L: LoopStrategy,
    T: TerminationStrategy,
{
    async fn dispatch(&self) -> Result<Value> {
        self.run().await
    }
}

/// Shared loop body used by both `Harness<L, T>::run` and
/// `BoxedHarness::run`. Takes `&dyn` references to the two strategy
/// traits so the typed form preserves monomorphization while the boxed
/// form re-uses the same code path through one indirect call per step.
pub(crate) async fn run_impl(
    spec: &HarnessSpec,
    loop_strategy: &dyn LoopStrategy,
    termination: &dyn TerminationStrategy,
    bus: &EventBus,
) -> Result<Value> {
    let mut state = HarnessState::new(spec.initial_budget);
    loop {
        // Termination check at top of loop covers `iteration >= cap`.
        if let Termination::Done(reason) = termination.should_terminate(&state) {
            bus.emit(Event::HarnessIteration {
                harness_id: spec.id.clone(),
                iteration: state.iteration,
                outcome: format!("terminated:{reason}"),
                budget_remaining_tokens: state.remaining_tokens,
            });
            return Ok(state.working_memory);
        }
        state.iteration += 1;
        let outcome = loop_strategy.step(&mut state).await?;
        match outcome {
            StepOutcome::Continue {
                working_memory,
                label,
            } => {
                state.working_memory = working_memory;
                state.history.push(StepEvent {
                    iteration: state.iteration,
                    outcome: label.clone(),
                    timestamp_ms: chrono::Utc::now().timestamp_millis(),
                });
                bus.emit(Event::HarnessIteration {
                    harness_id: spec.id.clone(),
                    iteration: state.iteration,
                    outcome: label,
                    budget_remaining_tokens: state.remaining_tokens,
                });
            }
            StepOutcome::Done { output, label } => {
                state.history.push(StepEvent {
                    iteration: state.iteration,
                    outcome: label.clone(),
                    timestamp_ms: chrono::Utc::now().timestamp_millis(),
                });
                bus.emit(Event::HarnessIteration {
                    harness_id: spec.id.clone(),
                    iteration: state.iteration,
                    outcome: format!("done:{label}"),
                    budget_remaining_tokens: state.remaining_tokens,
                });
                return Ok(output);
            }
        }
    }
}

#[allow(dead_code)]
fn _ensure_error_in_scope() -> AgentError {
    AgentError::Harness("placeholder".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::termination::IterationCapTermination;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    struct CountToThree {
        counter: Arc<AtomicU32>,
    }

    #[async_trait]
    impl LoopStrategy for CountToThree {
        async fn step(&self, _state: &mut HarnessState) -> Result<StepOutcome> {
            let v = self.counter.fetch_add(1, Ordering::SeqCst) + 1;
            if v >= 3 {
                Ok(StepOutcome::Done {
                    output: serde_json::json!(v),
                    label: "reached".into(),
                })
            } else {
                Ok(StepOutcome::Continue {
                    working_memory: serde_json::json!(v),
                    label: format!("step-{v}"),
                })
            }
        }
    }

    #[tokio::test]
    async fn harness_runs_until_done() {
        let counter = Arc::new(AtomicU32::new(0));
        let h = Harness {
            spec: HarnessSpec {
                id: HarnessId::from("count-to-three"),
                version: Version::new(0, 1, 0),
                eval_suite_id: None,
                initial_budget: TokenBudget::new(1000),
            },
            loop_strategy: CountToThree {
                counter: counter.clone(),
            },
            termination: IterationCapTermination { cap: 100 },
            bus: EventBus::new(),
        };
        let out = h.run().await.unwrap();
        assert_eq!(out, serde_json::json!(3));
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn harness_terminates_at_cap() {
        struct ForeverContinue;
        #[async_trait]
        impl LoopStrategy for ForeverContinue {
            async fn step(&self, _state: &mut HarnessState) -> Result<StepOutcome> {
                Ok(StepOutcome::Continue {
                    working_memory: Value::Null,
                    label: "tick".into(),
                })
            }
        }
        let h = Harness {
            spec: HarnessSpec {
                id: HarnessId::from("forever"),
                version: Version::new(0, 1, 0),
                eval_suite_id: None,
                initial_budget: TokenBudget::new(1000),
            },
            loop_strategy: ForeverContinue,
            termination: IterationCapTermination { cap: 5 },
            bus: EventBus::new(),
        };
        let _ = h.run().await.unwrap();
        // No assertion required beyond "doesn't loop forever".
    }

    #[tokio::test]
    async fn typed_harness_implements_dispatch() {
        // Confirm the HarnessDispatch impl on the typed form works.
        let counter = Arc::new(AtomicU32::new(0));
        let h = Harness {
            spec: HarnessSpec {
                id: HarnessId::from("dispatched"),
                version: Version::new(0, 1, 0),
                eval_suite_id: None,
                initial_budget: TokenBudget::new(1000),
            },
            loop_strategy: CountToThree {
                counter: counter.clone(),
            },
            termination: IterationCapTermination { cap: 100 },
            bus: EventBus::new(),
        };
        let dispatched: &dyn HarnessDispatch = &h;
        let out = dispatched.dispatch().await.unwrap();
        assert_eq!(out, serde_json::json!(3));
    }

    #[tokio::test]
    async fn into_boxed_runs_identically() {
        let counter = Arc::new(AtomicU32::new(0));
        let h = Harness {
            spec: HarnessSpec {
                id: HarnessId::from("boxed-via-into"),
                version: Version::new(0, 1, 0),
                eval_suite_id: None,
                initial_budget: TokenBudget::new(1000),
            },
            loop_strategy: CountToThree {
                counter: counter.clone(),
            },
            termination: IterationCapTermination { cap: 100 },
            bus: EventBus::new(),
        };
        let boxed = h.into_boxed();
        let out = boxed.run().await.unwrap();
        assert_eq!(out, serde_json::json!(3));
    }
}
