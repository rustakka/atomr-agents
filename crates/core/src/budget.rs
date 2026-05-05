use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::error::{AgentError, Result};

/// Token budget threaded through every strategy resolution. Strategies
/// `consume` from a shared budget; the `ContextAssembler` honors the
/// final cap when packing fragments.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TokenBudget {
    pub remaining: u32,
    pub reserved: u32,
}

impl TokenBudget {
    pub fn new(total: u32) -> Self {
        Self {
            remaining: total,
            reserved: 0,
        }
    }

    pub fn consume(&mut self, n: u32) -> Result<()> {
        if n > self.remaining {
            return Err(AgentError::BudgetExceeded("tokens"));
        }
        self.remaining -= n;
        Ok(())
    }

    pub fn reserve(&mut self, n: u32) -> Result<()> {
        if n > self.remaining {
            return Err(AgentError::BudgetExceeded("tokens"));
        }
        self.remaining -= n;
        self.reserved += n;
        Ok(())
    }

    pub fn release(&mut self, n: u32) {
        let n = n.min(self.reserved);
        self.reserved -= n;
        self.remaining += n;
    }

    /// Split the *current* remaining budget into `n` equal slices for
    /// cooperative parallel resolution. Each slice is independent;
    /// after the parallel join, the caller sums what was actually used
    /// and updates the parent.
    pub fn split(&self, n: u32) -> Vec<TokenBudget> {
        if n == 0 {
            return Vec::new();
        }
        let per = self.remaining / n;
        (0..n).map(|_| TokenBudget::new(per)).collect()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TimeBudget {
    /// Wall-clock budget remaining as milliseconds.
    pub remaining_ms: u64,
}

impl TimeBudget {
    pub fn new(d: Duration) -> Self {
        Self {
            remaining_ms: d.as_millis().min(u64::MAX as u128) as u64,
        }
    }

    pub fn consume(&mut self, d: Duration) -> Result<()> {
        let ms = d.as_millis() as u64;
        if ms > self.remaining_ms {
            return Err(AgentError::BudgetExceeded("time"));
        }
        self.remaining_ms -= ms;
        Ok(())
    }
}

/// Money budget. Stored as integer micro-USD to avoid float drift.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MoneyBudget {
    pub remaining_micro_usd: u64,
}

impl MoneyBudget {
    pub fn from_usd(usd: f64) -> Self {
        Self {
            remaining_micro_usd: (usd * 1_000_000.0) as u64,
        }
    }

    pub fn consume_micro(&mut self, micro: u64) -> Result<()> {
        if micro > self.remaining_micro_usd {
            return Err(AgentError::BudgetExceeded("money"));
        }
        self.remaining_micro_usd -= micro;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct IterationBudget {
    pub remaining: u32,
}

impl IterationBudget {
    pub fn new(n: u32) -> Self {
        Self { remaining: n }
    }

    pub fn consume_one(&mut self) -> Result<()> {
        if self.remaining == 0 {
            return Err(AgentError::BudgetExceeded("iterations"));
        }
        self.remaining -= 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_budget_consume_and_split() {
        let mut b = TokenBudget::new(1000);
        b.consume(100).unwrap();
        assert_eq!(b.remaining, 900);
        let parts = b.split(3);
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].remaining, 300);
    }

    #[test]
    fn budget_exceeded() {
        let mut b = TokenBudget::new(10);
        assert!(b.consume(11).is_err());
    }

    #[test]
    fn iteration_budget() {
        let mut b = IterationBudget::new(2);
        assert!(b.consume_one().is_ok());
        assert!(b.consume_one().is_ok());
        assert!(b.consume_one().is_err());
    }
}
