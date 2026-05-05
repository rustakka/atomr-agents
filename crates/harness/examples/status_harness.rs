//! `StatusHarness` reference example.
//!
//! A scheduled, mostly-deterministic loop that:
//!   1. polls a `signals` source,
//!   2. diffs vs. last published state,
//!   3. classifies the diff,
//!   4. drafts a one-line summary,
//!   5. publishes (in this example, prints).
//!
//! Smallest reference harness — intended as a template for any
//! cron/heartbeat-style automation.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{Result, TokenBudget, Value};
use atomr_agents_harness::{
    Harness, HarnessSpec, IterationCapTermination, LoopStrategy, StepOutcome,
};
use atomr_agents_observability::EventBus;
use parking_lot::Mutex;
use semver::Version;

/// Stand-in for "the world": maps `host -> health` and is mutated by
/// the test driver between iterations.
#[derive(Clone, Default)]
struct SignalSource {
    inner: Arc<Mutex<BTreeMap<String, String>>>,
}

impl SignalSource {
    fn snapshot(&self) -> BTreeMap<String, String> {
        self.inner.lock().clone()
    }

    fn set(&self, host: &str, health: &str) {
        self.inner.lock().insert(host.into(), health.into());
    }
}

struct StatusLoop {
    signals: SignalSource,
    last: Arc<Mutex<BTreeMap<String, String>>>,
    max_iters: u64,
}

#[async_trait]
impl LoopStrategy for StatusLoop {
    async fn step(
        &self,
        state: &mut atomr_agents_harness::HarnessState,
    ) -> Result<StepOutcome> {
        let now = self.signals.snapshot();
        let prev = self.last.lock().clone();
        let mut diffs = Vec::new();
        for (host, health) in &now {
            match prev.get(host) {
                Some(p) if p == health => {}
                Some(p) => diffs.push(format!("{host}: {p} -> {health}")),
                None => diffs.push(format!("{host}: + {health}")),
            }
        }
        for host in prev.keys() {
            if !now.contains_key(host) {
                diffs.push(format!("{host}: -"));
            }
        }
        *self.last.lock() = now.clone();
        let summary = if diffs.is_empty() {
            "no changes".to_string()
        } else {
            diffs.join("; ")
        };
        eprintln!("[status @ {}] {summary}", state.iteration);
        if state.iteration >= self.max_iters {
            Ok(StepOutcome::Done {
                output: Value::String(summary),
                label: "max-iters".into(),
            })
        } else {
            Ok(StepOutcome::Continue {
                working_memory: serde_json::json!({"summary": summary}),
                label: "tick".into(),
            })
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let signals = SignalSource::default();
    signals.set("api-1", "healthy");
    signals.set("api-2", "healthy");

    let harness = Harness {
        spec: HarnessSpec {
            id: "status-harness".into(),
            version: Version::new(0, 1, 0),
            eval_suite_id: None,
            initial_budget: TokenBudget::new(10_000),
        },
        loop_strategy: StatusLoop {
            signals: signals.clone(),
            last: Arc::new(Mutex::new(BTreeMap::new())),
            max_iters: 3,
        },
        termination: IterationCapTermination { cap: 10 },
        bus: EventBus::new(),
    };

    // First iteration sees fresh signals; second sees a state change.
    tokio::spawn({
        let signals = signals.clone();
        async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            signals.set("api-2", "degraded");
        }
    });

    let out = harness.run().await?;
    println!("status final: {out}");
    Ok(())
}
