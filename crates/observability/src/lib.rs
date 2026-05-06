//! Structured event taxonomy and emit channel.
//!
//! Phase 1 shipped a process-local `EventBus`. Phase R6 layers on a
//! `RunTree` builder + `Tracer` trait so events can be flattened into
//! LangSmith-style run trees and exported to JSONL or LangSmith-shaped
//! sinks.

mod run_tree;
mod tracer;

pub use run_tree::{RunKind, RunNode, RunTreeBuilder};
pub use tracer::{JsonlTracer, LangSmithTracer, StdoutTracer, Tracer, TracerSink};

use std::sync::Arc;

use atomr_agents_core::{Event, EventEnvelope, RunId};
use parking_lot::Mutex;

/// Process-local event bus.
#[derive(Clone)]
pub struct EventBus {
    inner: Arc<Inner>,
}

type SinkFn = Box<dyn Fn(&EventEnvelope) + Send + Sync>;

struct Inner {
    sinks: Mutex<Vec<SinkFn>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                sinks: Mutex::new(Vec::new()),
            }),
        }
    }

    pub fn subscribe<F>(&self, f: F)
    where
        F: Fn(&EventEnvelope) + Send + Sync + 'static,
    {
        self.inner.sinks.lock().push(Box::new(f));
    }

    /// Emit a bare event with no run-id metadata.
    pub fn emit(&self, event: Event) {
        let env = EventEnvelope::now(event);
        for sink in self.inner.sinks.lock().iter() {
            sink(&env);
        }
    }

    /// Emit an event tagged with run-id (and optionally parent).
    pub fn emit_run(&self, event: Event, run_id: RunId, parent: Option<RunId>) {
        let env = EventEnvelope::now(event).with_run(run_id, parent);
        for sink in self.inner.sinks.lock().iter() {
            sink(&env);
        }
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_core::AgentId;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn bus_dispatches_to_subscribers() {
        let bus = EventBus::new();
        let count = Arc::new(AtomicU32::new(0));
        {
            let count = count.clone();
            bus.subscribe(move |_| {
                count.fetch_add(1, Ordering::SeqCst);
            });
        }
        bus.emit(Event::AgentTurn {
            agent_id: AgentId::from("a-1"),
            input_tokens: 10,
            output_tokens: 5,
            reasoning_tokens: 0,
            cached_tokens: 0,
            finish_reason: None,
            elapsed_ms: 20,
        });
        bus.emit(Event::Backpressure {
            actor_path: "/user/team-1".into(),
            queued: 32,
            dropped: 0,
        });
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn emit_run_attaches_run_id() {
        let bus = EventBus::new();
        let captured = Arc::new(Mutex::new(Vec::<EventEnvelope>::new()));
        {
            let c = captured.clone();
            bus.subscribe(move |env| c.lock().push(env.clone()));
        }
        let run = RunId::from("r-1");
        bus.emit_run(
            Event::AgentTurn {
                agent_id: AgentId::from("a-1"),
                input_tokens: 1,
                output_tokens: 2,
                reasoning_tokens: 0,
                cached_tokens: 0,
                finish_reason: None,
                elapsed_ms: 3,
            },
            run.clone(),
            None,
        );
        let g = captured.lock();
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].run_id.as_ref().map(|r| r.as_str()), Some("r-1"));
    }
}
