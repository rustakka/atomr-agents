use async_trait::async_trait;
use atomr_agents_callable::Callable;
use atomr_agents_core::{AgentError, CallCtx, Event, HarnessId, Result, TokenBudget, Value};
use atomr_agents_observability::EventBus;
use semver::Version;

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
        let mut state = HarnessState::new(self.spec.initial_budget);
        loop {
            // Termination check at top of loop covers `iteration >= cap`.
            if let Termination::Done(reason) = self.termination.should_terminate(&state) {
                self.bus.emit(Event::HarnessIteration {
                    harness_id: self.spec.id.clone(),
                    iteration: state.iteration,
                    outcome: format!("terminated:{reason}"),
                    budget_remaining_tokens: state.remaining_tokens,
                });
                return Ok(state.working_memory);
            }
            state.iteration += 1;
            let outcome = self.loop_strategy.step(&mut state).await?;
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
                    self.bus.emit(Event::HarnessIteration {
                        harness_id: self.spec.id.clone(),
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
                    self.bus.emit(Event::HarnessIteration {
                        harness_id: self.spec.id.clone(),
                        iteration: state.iteration,
                        outcome: format!("done:{label}"),
                        budget_remaining_tokens: state.remaining_tokens,
                    });
                    return Ok(output);
                }
            }
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
}
