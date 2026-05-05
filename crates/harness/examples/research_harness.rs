//! `ResearchHarness` reference example.
//!
//! Loop: clarify → search → read → synthesize → identify-gaps →
//! repeat-or-done. Demonstrates harness-calls-harness composition by
//! delegating "deep-dive on topic X" to a child harness.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::{Callable, FnCallable};
use atomr_agents_core::{CallCtx, IterationBudget, MoneyBudget, Result, TimeBudget, TokenBudget, Value};
use atomr_agents_harness::{Harness, HarnessSpec, IterationCapTermination, LoopStrategy, StepOutcome};
use atomr_agents_observability::EventBus;
use parking_lot::Mutex;
use semver::Version;
use std::time::Duration;

/// Mock corpus the search step queries.
fn corpus_search(query: &str) -> Vec<String> {
    let docs = [
        (
            "rust",
            vec![
                "Rust is a systems programming language",
                "Cargo is the build tool",
            ],
        ),
        (
            "actor",
            vec!["Actors have mailboxes", "Akka pioneered the supervision tree"],
        ),
        (
            "agent",
            vec![
                "Agents compose strategies",
                "atomr-agents wires them on top of atomr",
            ],
        ),
    ];
    docs.iter()
        .filter(|(k, _)| query.to_lowercase().contains(*k))
        .flat_map(|(_, v)| v.iter().map(|s| (*s).to_string()))
        .collect()
}

#[derive(Clone, Default)]
struct Notes(Arc<Mutex<Vec<String>>>);

impl Notes {
    fn append(&self, s: String) {
        self.0.lock().push(s);
    }
    fn join(&self) -> String {
        self.0.lock().join("\n")
    }
    fn count(&self) -> usize {
        self.0.lock().len()
    }
}

struct ResearchLoop {
    topic: String,
    notes: Notes,
    /// Child harness invoked for "deep-dive" — illustrates
    /// harness-calls-harness composition.
    deep_dive: Arc<dyn Callable>,
}

#[async_trait]
impl LoopStrategy for ResearchLoop {
    async fn step(&self, state: &mut atomr_agents_harness::HarnessState) -> Result<StepOutcome> {
        match state.iteration {
            1 => {
                // 1. Clarify
                let clarification = format!("research target: {}", self.topic);
                self.notes.append(format!("clarify: {clarification}"));
                Ok(StepOutcome::Continue {
                    working_memory: serde_json::json!({"phase": "clarify"}),
                    label: "clarify".into(),
                })
            }
            2 => {
                // 2. Search
                let hits = corpus_search(&self.topic);
                self.notes.append(format!("search: {} hit(s)", hits.len()));
                for h in &hits {
                    self.notes.append(format!("  - {h}"));
                }
                Ok(StepOutcome::Continue {
                    working_memory: serde_json::json!({"phase": "search", "hits": hits.len()}),
                    label: "search".into(),
                })
            }
            3 => {
                // 3. Read + 4. Synthesize (combined for demo)
                let summary = format!(
                    "read+synthesize: integrated {} notes on {}",
                    self.notes.count(),
                    self.topic
                );
                self.notes.append(summary.clone());
                Ok(StepOutcome::Continue {
                    working_memory: serde_json::json!({"phase": "synthesize"}),
                    label: "synthesize".into(),
                })
            }
            4 => {
                // 5. Identify gaps + delegate deep-dive (sub-harness call).
                let ctx = CallCtx {
                    agent_id: None,
                    tokens: TokenBudget::new(2_000),
                    time: TimeBudget::new(Duration::from_secs(10)),
                    money: MoneyBudget::from_usd(0.10),
                    iterations: IterationBudget::new(5),
                    trace: vec!["deep-dive".into()],
                };
                let dive = self
                    .deep_dive
                    .call(serde_json::json!({"topic": self.topic.clone()}), ctx)
                    .await?;
                self.notes.append(format!("gap-fill via deep-dive: {dive}"));
                Ok(StepOutcome::Done {
                    output: Value::String(self.notes.join()),
                    label: "report-ready".into(),
                })
            }
            _ => unreachable!("loop scheduled to terminate by iteration 4"),
        }
    }
}

/// Build a tiny "deep-dive" sub-harness wrapped as a `Callable`. In a
/// real system this would itself be a `Harness`; we use a closure
/// here to keep the example tight.
fn build_deep_dive() -> Arc<dyn Callable> {
    Arc::new(FnCallable::labeled(
        "deep-dive",
        |input: Value, _ctx| async move {
            let topic = input.get("topic").and_then(|v| v.as_str()).unwrap_or("?");
            Ok(serde_json::json!(format!("[deep-dive] more on {topic}")))
        },
    ))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let notes = Notes::default();
    let harness = Harness {
        spec: HarnessSpec {
            id: "research-harness".into(),
            version: Version::new(0, 1, 0),
            eval_suite_id: None,
            initial_budget: TokenBudget::new(20_000),
        },
        loop_strategy: ResearchLoop {
            topic: "rust agent framework".into(),
            notes: notes.clone(),
            deep_dive: build_deep_dive(),
        },
        termination: IterationCapTermination { cap: 8 },
        bus: EventBus::new(),
    };
    let report = harness.run().await?;
    println!("=== research report ===");
    println!("{report}");
    Ok(())
}
