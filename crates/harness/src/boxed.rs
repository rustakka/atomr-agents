//! Fully type-erased harness — holds `Box<dyn LoopStrategy>` /
//! `Box<dyn TerminationStrategy>` so callers without compile-time access
//! to the concrete strategy types (Python config loaders, registry
//! wiring) can construct a runnable harness.
//!
//! Mirrors `atomr-agents-agent::BoxedAgent` (W3a). The body of `run` is
//! shared with `Harness<L, T>::run` via the free function
//! `crate::harness::run_impl` — both forms preserve the same loop
//! semantics, the boxed form just adds one indirect call per step.

use async_trait::async_trait;
use atomr_agents_callable::Callable;
use atomr_agents_core::{CallCtx, Result, Value};
use atomr_agents_observability::EventBus;

use crate::dispatch::HarnessDispatch;
use crate::harness::{run_impl, HarnessSpec};
use crate::loop_strategy::LoopStrategy;
use crate::termination::TerminationStrategy;

/// A harness whose strategy generics have been erased into trait
/// objects. Constructed via `HarnessSpec::into_harness` or
/// `Harness::into_boxed`.
pub struct BoxedHarness {
    pub spec: HarnessSpec,
    pub loop_strategy: Box<dyn LoopStrategy>,
    pub termination: Box<dyn TerminationStrategy>,
    pub bus: EventBus,
}

impl BoxedHarness {
    /// Drive the harness loop. Identical semantics to `Harness::run`.
    pub async fn run(&self) -> Result<Value> {
        run_impl(
            &self.spec,
            &*self.loop_strategy,
            &*self.termination,
            &self.bus,
        )
        .await
    }
}

#[async_trait]
impl HarnessDispatch for BoxedHarness {
    async fn dispatch(&self) -> Result<Value> {
        self.run().await
    }
}

#[async_trait]
impl Callable for BoxedHarness {
    async fn call(&self, _input: Value, _ctx: CallCtx) -> Result<Value> {
        self.run().await
    }

    fn label(&self) -> &str {
        self.spec.id.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loop_strategy::StepOutcome;
    use crate::state::HarnessState;
    use crate::termination::IterationCapTermination;
    use atomr_agents_core::{HarnessId, TokenBudget};
    use semver::Version;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    struct CountToTwo {
        counter: Arc<AtomicU32>,
    }

    #[async_trait]
    impl LoopStrategy for CountToTwo {
        async fn step(&self, _state: &mut HarnessState) -> Result<StepOutcome> {
            let v = self.counter.fetch_add(1, Ordering::SeqCst) + 1;
            if v >= 2 {
                Ok(StepOutcome::Done {
                    output: serde_json::json!(v),
                    label: "done".into(),
                })
            } else {
                Ok(StepOutcome::Continue {
                    working_memory: serde_json::json!(v),
                    label: format!("tick-{v}"),
                })
            }
        }
    }

    #[tokio::test]
    async fn boxed_harness_end_to_end() {
        let counter = Arc::new(AtomicU32::new(0));
        let spec = HarnessSpec {
            id: HarnessId::from("boxed-e2e"),
            version: Version::new(0, 1, 0),
            eval_suite_id: None,
            initial_budget: TokenBudget::new(1000),
        };
        let bh = BoxedHarness {
            spec,
            loop_strategy: Box::new(CountToTwo {
                counter: counter.clone(),
            }),
            termination: Box::new(IterationCapTermination { cap: 100 }),
            bus: EventBus::new(),
        };
        let out = bh.run().await.unwrap();
        assert_eq!(out, serde_json::json!(2));
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn into_harness_returns_runnable_ref() {
        let counter = Arc::new(AtomicU32::new(0));
        let spec = HarnessSpec {
            id: HarnessId::from("via-spec"),
            version: Version::new(0, 1, 0),
            eval_suite_id: None,
            initial_budget: TokenBudget::new(1000),
        };
        let href = spec.into_harness(
            Box::new(CountToTwo {
                counter: counter.clone(),
            }),
            Box::new(IterationCapTermination { cap: 100 }),
        );
        assert_eq!(href.id.as_str(), "via-spec");
        let out = href.run().await.unwrap();
        assert_eq!(out, serde_json::json!(2));
    }

    #[tokio::test]
    async fn boxed_harness_terminates_at_cap() {
        struct Forever;
        #[async_trait]
        impl LoopStrategy for Forever {
            async fn step(&self, _state: &mut HarnessState) -> Result<StepOutcome> {
                Ok(StepOutcome::Continue {
                    working_memory: Value::Null,
                    label: "tick".into(),
                })
            }
        }
        let spec = HarnessSpec {
            id: HarnessId::from("boxed-cap"),
            version: Version::new(0, 1, 0),
            eval_suite_id: None,
            initial_budget: TokenBudget::new(1000),
        };
        let bh = BoxedHarness {
            spec,
            loop_strategy: Box::new(Forever),
            termination: Box::new(IterationCapTermination { cap: 4 }),
            bus: EventBus::new(),
        };
        let _ = bh.run().await.unwrap();
        // Implicit assertion: the cap was respected (test would hang
        // otherwise).
    }
}
