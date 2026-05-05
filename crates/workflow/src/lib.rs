//! Workflow engine. DAG of `Step`s; state is event-sourced and
//! resumable.

mod dag;
mod dispatch;
mod event;
mod interrupt;
mod runner;
mod state_runner;
mod step;
mod subgraph;

pub use dispatch::dispatch_fan_out;
pub use subgraph::Subgraph;

pub use dag::{Dag, StepId};
pub use event::{InMemoryJournal, Journal, WorkflowEvent};
pub use interrupt::{
    Command, FnInterruptStep, InterruptCtrl, Interruptible, InterruptibleStep, PauseReason,
    PlainStep, RunOutcome,
};
pub use runner::{WorkflowRunner, WorkflowState};
pub use state_runner::{FnStatefulStep, StatefulRunner, StatefulStep};
pub use step::{
    BranchPredicate, Concurrency, HumanApproval, InputMapping, JoinStrategy, Step,
};
