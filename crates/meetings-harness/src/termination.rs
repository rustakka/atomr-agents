//! Termination strategies for the meetings harness loop.

use crate::state::MeetingsHarnessState;

/// Outcome of a termination check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Termination {
    Continue,
    Done(&'static str),
}

/// Decides whether the meetings loop should stop.
pub trait MeetingsTermination: Send + Sync + 'static {
    fn should_terminate(&self, state: &MeetingsHarnessState) -> Termination;
}

impl MeetingsTermination for Box<dyn MeetingsTermination> {
    fn should_terminate(&self, state: &MeetingsHarnessState) -> Termination {
        (**self).should_terminate(state)
    }
}

/// Stop only when the loop strategy returns `Done`. Default for batch
/// runs where the strategy controls the lifecycle.
#[derive(Debug, Default, Clone, Copy)]
pub struct StreamEndTermination;

impl MeetingsTermination for StreamEndTermination {
    fn should_terminate(&self, state: &MeetingsHarnessState) -> Termination {
        if state.cancel_requested {
            Termination::Done("cancelled")
        } else if state.stream_closed {
            Termination::Done("stream_end")
        } else {
            Termination::Continue
        }
    }
}

/// Stop once `cap` iterations have run.
#[derive(Debug, Clone, Copy)]
pub struct IterationCapTermination {
    pub cap: u32,
}

impl IterationCapTermination {
    pub fn new(cap: u32) -> Self {
        Self { cap }
    }
}

impl MeetingsTermination for IterationCapTermination {
    fn should_terminate(&self, state: &MeetingsHarnessState) -> Termination {
        if state.iteration >= self.cap as u64 {
            Termination::Done("iteration_cap")
        } else if state.cancel_requested {
            Termination::Done("cancelled")
        } else {
            Termination::Continue
        }
    }
}

/// Stop once the token-shaped budget proxy is exhausted.
#[derive(Debug, Default, Clone, Copy)]
pub struct BudgetTermination;

impl MeetingsTermination for BudgetTermination {
    fn should_terminate(&self, state: &MeetingsHarnessState) -> Termination {
        if state.cancel_requested {
            Termination::Done("cancelled")
        } else if state.remaining_budget == 0 {
            Termination::Done("budget_exhausted")
        } else {
            Termination::Continue
        }
    }
}

/// OR-combine several strategies: the first to fire wins.
pub struct CompositeTermination(pub Vec<Box<dyn MeetingsTermination>>);

impl CompositeTermination {
    pub fn new(strategies: Vec<Box<dyn MeetingsTermination>>) -> Self {
        Self(strategies)
    }
}

impl MeetingsTermination for CompositeTermination {
    fn should_terminate(&self, state: &MeetingsHarnessState) -> Termination {
        for strat in &self.0 {
            if let Termination::Done(reason) = strat.should_terminate(state) {
                return Termination::Done(reason);
            }
        }
        Termination::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::MeetingAnalysis;
    use atomr_agents_stt_harness::SttConversation;

    fn state(iter: u64, budget: u32) -> MeetingsHarnessState {
        let mut s = MeetingsHarnessState::new(SttConversation::new("c1"), budget);
        s.iteration = iter;
        s.analysis = MeetingAnalysis::new("c1");
        s
    }

    #[test]
    fn iteration_cap_fires_at_cap() {
        let t = IterationCapTermination::new(3);
        assert_eq!(t.should_terminate(&state(2, 0)), Termination::Continue);
        assert_eq!(t.should_terminate(&state(3, 0)), Termination::Done("iteration_cap"));
    }

    #[test]
    fn cancellation_fires_through_any_strategy() {
        let mut s = state(0, 100);
        s.cancel_requested = true;
        assert_eq!(StreamEndTermination.should_terminate(&s), Termination::Done("cancelled"));
        assert_eq!(BudgetTermination.should_terminate(&s), Termination::Done("cancelled"));
        assert_eq!(
            IterationCapTermination::new(100).should_terminate(&s),
            Termination::Done("cancelled")
        );
    }

    #[test]
    fn composite_first_to_fire_wins() {
        let t = CompositeTermination::new(vec![
            Box::new(IterationCapTermination::new(10)),
            Box::new(BudgetTermination),
        ]);
        // budget exhausted, iter low → budget fires
        let s = state(2, 0);
        assert_eq!(t.should_terminate(&s), Termination::Done("budget_exhausted"));
        // both quiet → continue
        let s = state(2, 100);
        assert_eq!(t.should_terminate(&s), Termination::Continue);
    }
}
