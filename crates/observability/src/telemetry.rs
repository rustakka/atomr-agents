//! Telemetry fan-out: a [`Telemetry`] handle holds a set of
//! [`TelemetrySink`]s and broadcasts every [`RunEvent`] to all of them.
//!
//! Two ways events reach the sinks:
//! * **Directly** — the model-drift / interrupt / cost / trigger
//!   features call [`Telemetry::emit`] with a fully-formed `RunEvent`
//!   (carrying its `checkpoint_ref`).
//! * **Bridged** — [`Telemetry::bridge_from`] subscribes to a process
//!   [`crate::EventBus`] and projects the in-process `Event` taxonomy
//!   into `RunEvent`s so existing call sites surface on the backbone
//!   too.
//!
//! Sinks ship for the common targets: an in-memory collector (tests), a
//! JSONL writer, and an unbounded channel ([`ChannelTelemetrySink`])
//! that a downstream `atomr-streams` Source can wrap for reactive,
//! back-pressured consumption.

use std::sync::Arc;

use atomr_agents_core::{Event, EventEnvelope};
use parking_lot::Mutex;
use tokio::sync::mpsc;

use crate::run_event::{RunEvent, RunEventKind, TokenIo};
use crate::EventBus;

/// A consumer of [`RunEvent`]s. Mirrors the sync `EventBus` sink shape so
/// emission never blocks the run loop.
pub trait TelemetrySink: Send + Sync + 'static {
    fn emit(&self, event: &RunEvent);
}

/// Fan-out handle. Cloneable and cheap to pass into builders.
#[derive(Clone, Default)]
pub struct Telemetry {
    sinks: Arc<Mutex<Vec<Arc<dyn TelemetrySink>>>>,
}

impl Telemetry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a sink. Returns `self` for chaining.
    pub fn with_sink(self, sink: Arc<dyn TelemetrySink>) -> Self {
        self.sinks.lock().push(sink);
        self
    }

    /// Register a sink in place.
    pub fn add_sink(&self, sink: Arc<dyn TelemetrySink>) {
        self.sinks.lock().push(sink);
    }

    /// Broadcast a fully-formed event to every sink.
    pub fn emit(&self, event: RunEvent) {
        for sink in self.sinks.lock().iter() {
            sink.emit(&event);
        }
    }

    /// Subscribe to a process [`EventBus`]; each in-process `Event` that
    /// has a `RunEvent` projection is forwarded to the sinks.
    pub fn bridge_from(&self, bus: &EventBus) {
        let this = self.clone();
        bus.subscribe(move |env| {
            if let Some(re) = project_event(env) {
                this.emit(re);
            }
        });
    }
}

/// Project an in-process [`EventEnvelope`] into a [`RunEvent`], when a
/// meaningful mapping exists. Events with no run-level meaning (e.g.
/// `Backpressure`) return `None`.
pub fn project_event(env: &EventEnvelope) -> Option<RunEvent> {
    let run = env.run_id.as_ref().map(|r| r.as_str().to_string());
    let mk = |kind: RunEventKind| {
        let mut e = RunEvent::new(kind);
        e.run = run.clone();
        e.ts = env.timestamp_ms;
        e
    };
    match &env.event {
        Event::ToolCallStreamed { tool_name, .. } => Some(mk(RunEventKind::ToolDispatched {
            tool: tool_name.clone(),
        })),
        Event::ToolInvoked { tool_id, ok, .. } => Some(mk(RunEventKind::ToolReturned {
            tool: tool_id.as_str().to_string(),
            ok: *ok,
        })),
        Event::AgentTurn {
            input_tokens,
            output_tokens,
            reasoning_tokens,
            cached_tokens,
            ..
        } => {
            let mut e = mk(RunEventKind::InferenceCompleted);
            e.tokens = Some(TokenIo {
                input: *input_tokens,
                output: *output_tokens,
                reasoning: *reasoning_tokens,
                cached: *cached_tokens,
            });
            Some(e)
        }
        Event::WorkflowStep { workflow_id, ok, .. } if *ok => {
            let mut e = mk(RunEventKind::CheckpointCreated);
            e.workflow = Some(workflow_id.as_str().to_string());
            Some(e)
        }
        _ => None,
    }
}

/// Collects events in memory. Useful for assertions in tests and for a
/// UI that polls a recent-events buffer.
#[derive(Default, Clone)]
pub struct InMemoryTelemetrySink {
    events: Arc<Mutex<Vec<RunEvent>>>,
}

impl InMemoryTelemetrySink {
    pub fn new() -> Self {
        Self::default()
    }
    /// Snapshot of all events received so far.
    pub fn events(&self) -> Vec<RunEvent> {
        self.events.lock().clone()
    }
    pub fn len(&self) -> usize {
        self.events.lock().len()
    }
    pub fn is_empty(&self) -> bool {
        self.events.lock().is_empty()
    }
}

impl TelemetrySink for InMemoryTelemetrySink {
    fn emit(&self, event: &RunEvent) {
        self.events.lock().push(event.clone());
    }
}

/// Pushes each event as a JSON line to an underlying writer-callback.
/// The callback abstracts the file/stdout/transport so this stays
/// dependency-light.
pub struct JsonlTelemetrySink {
    write_line: Box<dyn Fn(String) + Send + Sync>,
}

impl JsonlTelemetrySink {
    pub fn new(write_line: impl Fn(String) + Send + Sync + 'static) -> Self {
        Self {
            write_line: Box::new(write_line),
        }
    }
}

impl TelemetrySink for JsonlTelemetrySink {
    fn emit(&self, event: &RunEvent) {
        if let Ok(line) = serde_json::to_string(event) {
            (self.write_line)(line);
        }
    }
}

/// Forwards events onto an unbounded channel. The receiver can be
/// wrapped by an `atomr-streams` Source so a downstream consumer (e.g.
/// hedgehog's visibility projector) ingests events as a reactive,
/// back-pressured stream.
pub struct ChannelTelemetrySink {
    tx: mpsc::UnboundedSender<RunEvent>,
}

impl ChannelTelemetrySink {
    /// Create the sink and its receiver half.
    pub fn new() -> (Self, mpsc::UnboundedReceiver<RunEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self { tx }, rx)
    }
}

impl TelemetrySink for ChannelTelemetrySink {
    fn emit(&self, event: &RunEvent) {
        // Best-effort: if the receiver is gone, drop the event.
        let _ = self.tx.send(event.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_event::CheckpointRef;
    use atomr_agents_core::{AgentId, Event, RunId};

    #[test]
    fn direct_emit_reaches_sinks_with_checkpoint_ref() {
        let mem = Arc::new(InMemoryTelemetrySink::new());
        let tel = Telemetry::new().with_sink(mem.clone());

        tel.emit(
            RunEvent::new(RunEventKind::CheckpointCreated)
                .with_checkpoint(CheckpointRef::new("wf-1", "run-1", 3)),
        );

        let events = mem.events();
        assert_eq!(events.len(), 1);
        let cp = events[0].checkpoint_ref.as_ref().unwrap();
        assert_eq!(cp.super_step, 3);
        assert_eq!(events[0].run.as_deref(), Some("run-1"));
    }

    #[test]
    fn bridge_projects_agent_turn_into_inference_completed() {
        let mem = Arc::new(InMemoryTelemetrySink::new());
        let tel = Telemetry::new().with_sink(mem.clone());
        let bus = EventBus::new();
        tel.bridge_from(&bus);

        bus.emit_run(
            Event::AgentTurn {
                agent_id: AgentId::from("a-1"),
                input_tokens: 10,
                output_tokens: 4,
                reasoning_tokens: 1,
                cached_tokens: 2,
                finish_reason: None,
                elapsed_ms: 5,
            },
            RunId::from("run-9"),
            None,
        );

        let events = mem.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, RunEventKind::InferenceCompleted);
        let toks = events[0].tokens.unwrap();
        assert_eq!(toks.input, 10);
        assert_eq!(toks.cached, 2);
        assert_eq!(events[0].run.as_deref(), Some("run-9"));
    }

    #[test]
    fn backpressure_event_has_no_projection() {
        let env = EventEnvelope::now(Event::Backpressure {
            actor_path: "/x".into(),
            queued: 1,
            dropped: 0,
        });
        assert!(project_event(&env).is_none());
    }

    #[tokio::test]
    async fn channel_sink_delivers_to_receiver() {
        let (sink, mut rx) = ChannelTelemetrySink::new();
        let tel = Telemetry::new().with_sink(Arc::new(sink));
        tel.emit(RunEvent::new(RunEventKind::InferenceCompleted));
        let got = rx.recv().await.unwrap();
        assert_eq!(got.kind, RunEventKind::InferenceCompleted);
    }
}
