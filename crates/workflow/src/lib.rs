//! Workflow engine. DAG of `Step`s; state is event-sourced and
//! resumable.

mod contract;
mod dag;
mod dispatch;
mod event;
mod interrupt;
mod interrupt_registry;
mod replay_sink;
mod resume_effect;
mod runner;
mod state_runner;
mod step;
mod subgraph;

pub use dispatch::dispatch_fan_out;
pub use subgraph::Subgraph;

pub use dag::{Dag, StepId};
pub use event::{InMemoryJournal, Journal, WorkflowEvent};
pub use interrupt::{
    Command, FnInterruptStep, InterruptCtrl, Interruptible, InterruptibleStep, PauseReason, PlainStep,
    RunOutcome,
};
pub use runner::{WorkflowRunner, WorkflowState};
pub use state_runner::{FnStatefulStep, StatefulRunner, StatefulStep};
pub use step::{BranchPredicate, Concurrency, HumanApproval, InputMapping, JoinStrategy, Step};

// FR-4 — Durable, queryable, fleet-wide HITL interrupt registry.
pub use interrupt_registry::{
    EscalationPolicy, InMemoryInterruptRegistry, InterruptFilter, InterruptIdGen, InterruptRegistry,
    InterruptStatus, PendingInterrupt, RegistryError, Resolution,
};
#[cfg(feature = "sql")]
pub use interrupt_registry::SqlInterruptRegistry;

// FR-5 — Transactional HITL resume / outbox.
pub use resume_effect::{
    drive_outbox, resume_with, InMemoryOutboxStore, OutboxRecord, OutboxSink, OutboxStore, ResumeEffect,
    SagaStep,
};
#[cfg(feature = "sql")]
pub use resume_effect::{resume_with_sql, SqlOutboxStore};

// FR-2 — Deterministic replay sink.
pub use replay_sink::{replay_to_sink, ChannelReplaySink, DecisionReplaySink, StepRef};

// FR-15 — Contract-validate pipeline operator.
pub use contract::{
    BreachSink, ContractValidateStage, ContractViolation, CustomCheck, DataContract, ExpectedCadence,
    InterruptSpec, JsonType, MaxLag, OnBreach, StageOutcome, TypeSchema, ViolationKind,
};
