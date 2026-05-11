//! Harness — tested, packaged, persistent execution loop.

mod boxed;
mod dispatch;
mod harness;
mod loop_strategy;
mod state;
mod termination;

pub use boxed::BoxedHarness;
pub use dispatch::{HarnessDispatch, HarnessRef};
pub use harness::{Harness, HarnessSpec};
pub use loop_strategy::{LoopStrategy, StepOutcome};
pub use state::{HarnessState, StepEvent};
pub use termination::{IterationCapTermination, Termination, TerminationStrategy};

/// Re-export for convenience: every harness is a `Callable`.
pub use atomr_agents_callable::{Callable, CallableHandle};
