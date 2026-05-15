//! Termination strategies.

use crate::state::DeepResearchState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Termination {
    Continue,
    Done(&'static str),
}

pub trait DeepResearchTermination: Send + Sync + 'static {
    fn should_terminate(&self, state: &DeepResearchState) -> Termination;
}

impl DeepResearchTermination for Box<dyn DeepResearchTermination> {
    fn should_terminate(&self, state: &DeepResearchState) -> Termination {
        (**self).should_terminate(state)
    }
}

/// Stop after `cap` iterations.
#[derive(Debug, Clone, Copy)]
pub struct IterationCapTermination {
    pub cap: u32,
}

impl IterationCapTermination {
    pub fn new(cap: u32) -> Self {
        Self { cap }
    }
}

impl DeepResearchTermination for IterationCapTermination {
    fn should_terminate(&self, state: &DeepResearchState) -> Termination {
        if state.cancel_requested {
            Termination::Done("cancelled")
        } else if state.iteration >= self.cap as u64 {
            Termination::Done("iteration_cap")
        } else {
            Termination::Continue
        }
    }
}

/// Stop only when the loop strategy itself returns `Done`.
#[derive(Debug, Default, Clone, Copy)]
pub struct StrategyControlledTermination;

impl DeepResearchTermination for StrategyControlledTermination {
    fn should_terminate(&self, state: &DeepResearchState) -> Termination {
        if state.cancel_requested {
            Termination::Done("cancelled")
        } else {
            Termination::Continue
        }
    }
}

/// Stop once the token budget is exhausted.
#[derive(Debug, Default, Clone, Copy)]
pub struct BudgetTermination;

impl DeepResearchTermination for BudgetTermination {
    fn should_terminate(&self, state: &DeepResearchState) -> Termination {
        if state.cancel_requested {
            Termination::Done("cancelled")
        } else if state.remaining_budget == 0 {
            Termination::Done("budget_exhausted")
        } else {
            Termination::Continue
        }
    }
}

/// OR-combine several strategies.
pub struct CompositeTermination(pub Vec<Box<dyn DeepResearchTermination>>);

impl CompositeTermination {
    pub fn new(strategies: Vec<Box<dyn DeepResearchTermination>>) -> Self {
        Self(strategies)
    }
}

impl DeepResearchTermination for CompositeTermination {
    fn should_terminate(&self, state: &DeepResearchState) -> Termination {
        for s in &self.0 {
            if let Termination::Done(reason) = s.should_terminate(state) {
                return Termination::Done(reason);
            }
        }
        Termination::Continue
    }
}
