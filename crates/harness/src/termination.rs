use crate::state::HarnessState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Termination {
    Continue,
    Done(&'static str),
}

pub trait TerminationStrategy: Send + Sync + 'static {
    fn should_terminate(&self, state: &HarnessState) -> Termination;
}

/// Stop after `cap` iterations.
pub struct IterationCapTermination {
    pub cap: u64,
}

impl TerminationStrategy for IterationCapTermination {
    fn should_terminate(&self, state: &HarnessState) -> Termination {
        if state.iteration >= self.cap {
            Termination::Done("iteration_cap")
        } else {
            Termination::Continue
        }
    }
}
