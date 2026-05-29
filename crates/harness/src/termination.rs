use atomr_agents_agent::{Budget, SpendLedger};

use crate::state::HarnessState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Termination {
    Continue,
    Done(&'static str),
}

pub trait TerminationStrategy: Send + Sync + 'static {
    fn should_terminate(&self, state: &HarnessState) -> Termination;
}

impl TerminationStrategy for Box<dyn TerminationStrategy> {
    fn should_terminate(&self, state: &HarnessState) -> Termination {
        (**self).should_terminate(state)
    }
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

/// Terminate a run cleanly when a [`Budget`] is exceeded (FR-18). Holds a
/// shared [`SpendLedger`] (cloneable handle) that the model `Callable`'s
/// `CostMeter` feeds; on each iteration it checks the budget at its scope
/// and stops the run on overspend instead of letting it burn money.
pub struct BudgetTermination {
    ledger: SpendLedger,
    budget: Budget,
}

impl BudgetTermination {
    pub fn new(ledger: SpendLedger, budget: Budget) -> Self {
        Self { ledger, budget }
    }
}

impl TerminationStrategy for BudgetTermination {
    fn should_terminate(&self, _state: &HarnessState) -> Termination {
        match self.ledger.check(&self.budget) {
            Ok(()) => Termination::Continue,
            Err(_) => Termination::Done("budget_exceeded"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_agent::{BudgetScope, Cap, DecisionKey, Spend};

    #[test]
    fn budget_termination_stops_on_overspend() {
        let ledger = SpendLedger::new();
        let budget = Budget {
            cap: Cap::Money(10_000),
            scope: BudgetScope::Global,
        };
        let term = BudgetTermination::new(ledger.clone(), budget);
        let state = HarnessState::new(atomr_agents_core::TokenBudget::new(1000));
        assert_eq!(term.should_terminate(&state), Termination::Continue);

        ledger.record(&Spend {
            micro_usd: 12_000,
            tokens: 10,
            decision_key: Some(DecisionKey::new("d", "s", "1")),
        });
        assert_eq!(
            term.should_terminate(&state),
            Termination::Done("budget_exceeded")
        );
    }
}
