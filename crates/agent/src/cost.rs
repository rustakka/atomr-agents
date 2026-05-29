//! Cost / budget enforcement + spend-to-decision-key attribution (FR-18).
//!
//! A fund must hard-cap inference spend per desk/strategy and kill runs
//! on overspend, and continuously know whether each decision earns more
//! than it costs to compute. This module provides:
//!
//! * [`CostMeter`] — pre-flight [`CostMeter::estimate`] and post-call
//!   [`CostMeter::record`] over a [`Pricing`] table;
//! * [`DecisionKey`] — carried on [`CallCtx`](atomr_agents_core::CallCtx)
//!   via the typed extension map (no struct change) so every [`Spend`] is
//!   attributable;
//! * [`SpendLedger`] — totals spend by decision key (and globally), and
//!   emits [`RunEventKind::BudgetSpent`] onto the telemetry backbone;
//! * [`Budget`] / [`BudgetExceeded`] — a cap at a [`BudgetScope`] that a
//!   `TerminationStrategy` can consult to terminate a run cleanly.

use std::collections::HashMap;
use std::sync::Arc;

use atomr_agents_observability::{RunEvent, RunEventKind, Telemetry};
use atomr_infer_core::tokens::TokenUsage;
use parking_lot::Mutex;
use thiserror::Error;

/// Business attribution for spend. Carried on `CallCtx` as a typed
/// extension: `ctx.insert_ext(DecisionKey::new(...))`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DecisionKey {
    pub desk: String,
    pub strategy: String,
    pub decision_id: String,
}

impl DecisionKey {
    pub fn new(desk: impl Into<String>, strategy: impl Into<String>, decision_id: impl Into<String>) -> Self {
        Self {
            desk: desk.into(),
            strategy: strategy.into(),
            decision_id: decision_id.into(),
        }
    }
    /// Canonical string form `desk/strategy/decision_id`.
    pub fn canonical(&self) -> String {
        format!("{}/{}/{}", self.desk, self.strategy, self.decision_id)
    }
}

/// Per-model pricing in micro-USD per 1000 tokens (avoids float drift).
#[derive(Debug, Clone, Copy, Default)]
pub struct ModelPricing {
    pub input_micro_usd_per_1k: u64,
    pub output_micro_usd_per_1k: u64,
}

/// Pricing table keyed by model id.
#[derive(Debug, Clone, Default)]
pub struct Pricing {
    table: HashMap<String, ModelPricing>,
}

impl Pricing {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_model(mut self, model_id: impl Into<String>, pricing: ModelPricing) -> Self {
        self.table.insert(model_id.into(), pricing);
        self
    }
    fn get(&self, model_id: &str) -> ModelPricing {
        self.table.get(model_id).copied().unwrap_or_default()
    }
}

/// A pre-flight cost estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CostEstimate {
    pub micro_usd: u64,
    pub tokens: u32,
}

/// A realized spend after a call.
#[derive(Debug, Clone)]
pub struct Spend {
    pub micro_usd: u64,
    pub tokens: u32,
    pub decision_key: Option<DecisionKey>,
}

/// Estimates and records inference spend.
pub struct CostMeter {
    pricing: Pricing,
}

impl CostMeter {
    pub fn new(pricing: Pricing) -> Self {
        Self { pricing }
    }

    /// Pre-flight estimate from token estimates.
    pub fn estimate(&self, model_id: &str, est_input: u32, est_output: u32) -> CostEstimate {
        let p = self.pricing.get(model_id);
        let micro = (est_input as u64 * p.input_micro_usd_per_1k) / 1000
            + (est_output as u64 * p.output_micro_usd_per_1k) / 1000;
        CostEstimate {
            micro_usd: micro,
            tokens: est_input + est_output,
        }
    }

    /// Post-call spend from actual usage.
    pub fn record(&self, model_id: &str, usage: &TokenUsage, key: Option<DecisionKey>) -> Spend {
        let p = self.pricing.get(model_id);
        let micro = (usage.input_tokens as u64 * p.input_micro_usd_per_1k) / 1000
            + (usage.output_tokens as u64 * p.output_micro_usd_per_1k) / 1000;
        Spend {
            micro_usd: micro,
            tokens: usage.input_tokens + usage.output_tokens,
            decision_key: key,
        }
    }
}

/// Scope a budget applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetScope {
    Desk(String),
    Strategy(String),
    Global,
}

/// A spend cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cap {
    Money(u64), // micro-USD
    Tokens(u32),
}

/// A budget cap at a scope.
#[derive(Debug, Clone)]
pub struct Budget {
    pub cap: Cap,
    pub scope: BudgetScope,
}

/// Signalled when a budget is exceeded; a `TerminationStrategy` maps this
/// to a clean run termination.
#[derive(Debug, Clone, Error)]
#[error("budget exceeded for scope {scope:?}: {spent} > cap")]
pub struct BudgetExceeded {
    pub scope: BudgetScope,
    pub spent: u64,
}

#[derive(Default)]
struct LedgerInner {
    by_key: HashMap<String, (u64, u32)>, // canonical key -> (micro, tokens)
    by_desk: HashMap<String, (u64, u32)>,
    by_strategy: HashMap<String, (u64, u32)>,
    total_micro: u64,
    total_tokens: u32,
}

/// Accumulates spend for cost-vs-alpha analysis and budget checks.
#[derive(Clone, Default)]
pub struct SpendLedger {
    inner: Arc<Mutex<LedgerInner>>,
    telemetry: Option<Telemetry>,
    run_id: Option<String>,
}

impl SpendLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_telemetry(mut self, telemetry: Telemetry, run_id: impl Into<String>) -> Self {
        self.telemetry = Some(telemetry);
        self.run_id = Some(run_id.into());
        self
    }

    /// Record a spend, updating all roll-ups and emitting telemetry.
    pub fn record(&self, spend: &Spend) {
        {
            let mut g = self.inner.lock();
            g.total_micro += spend.micro_usd;
            g.total_tokens += spend.tokens;
            if let Some(k) = &spend.decision_key {
                let e = g.by_key.entry(k.canonical()).or_default();
                e.0 += spend.micro_usd;
                e.1 += spend.tokens;
                let d = g.by_desk.entry(k.desk.clone()).or_default();
                d.0 += spend.micro_usd;
                d.1 += spend.tokens;
                let s = g.by_strategy.entry(k.strategy.clone()).or_default();
                s.0 += spend.micro_usd;
                s.1 += spend.tokens;
            }
        }
        if let Some(t) = &self.telemetry {
            let mut e = RunEvent::new(RunEventKind::BudgetSpent {
                decision_key: spend.decision_key.as_ref().map(|k| k.canonical()),
                micro_usd: spend.micro_usd,
            });
            e.run = self.run_id.clone();
            t.emit(e);
        }
    }

    /// Total spend (micro-USD) attributed to a decision key.
    pub fn spend_by(&self, key: &DecisionKey) -> u64 {
        self.inner
            .lock()
            .by_key
            .get(&key.canonical())
            .map(|(m, _)| *m)
            .unwrap_or(0)
    }

    pub fn total_micro_usd(&self) -> u64 {
        self.inner.lock().total_micro
    }
    pub fn total_tokens(&self) -> u32 {
        self.inner.lock().total_tokens
    }

    fn scope_spend(&self, scope: &BudgetScope) -> (u64, u32) {
        let g = self.inner.lock();
        match scope {
            BudgetScope::Global => (g.total_micro, g.total_tokens),
            BudgetScope::Desk(d) => g.by_desk.get(d).copied().unwrap_or_default(),
            BudgetScope::Strategy(s) => g.by_strategy.get(s).copied().unwrap_or_default(),
        }
    }

    /// Check a budget against current spend at its scope.
    pub fn check(&self, budget: &Budget) -> std::result::Result<(), BudgetExceeded> {
        let (micro, tokens) = self.scope_spend(&budget.scope);
        let over = match budget.cap {
            Cap::Money(cap) => {
                if micro > cap {
                    Some(micro)
                } else {
                    None
                }
            }
            Cap::Tokens(cap) => {
                if tokens > cap {
                    Some(tokens as u64)
                } else {
                    None
                }
            }
        };
        match over {
            Some(spent) => Err(BudgetExceeded {
                scope: budget.scope.clone(),
                spent,
            }),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_observability::InMemoryTelemetrySink;

    fn pricing() -> Pricing {
        Pricing::new().with_model(
            "claude",
            ModelPricing {
                input_micro_usd_per_1k: 3000,   // $0.003 / 1k
                output_micro_usd_per_1k: 15000, // $0.015 / 1k
            },
        )
    }

    #[test]
    fn estimate_and_record() {
        let m = CostMeter::new(pricing());
        let est = m.estimate("claude", 1000, 1000);
        assert_eq!(est.micro_usd, 3000 + 15000);
        let usage = TokenUsage {
            input_tokens: 2000,
            output_tokens: 500,
            ..Default::default()
        };
        let spend = m.record("claude", &usage, Some(DecisionKey::new("credit", "carry", "d1")));
        assert_eq!(spend.micro_usd, 2 * 3000 + (500 * 15000) / 1000);
    }

    #[test]
    fn ledger_attributes_and_emits() {
        let sink = Arc::new(InMemoryTelemetrySink::new());
        let tel = Telemetry::new().with_sink(sink.clone());
        let ledger = SpendLedger::new().with_telemetry(tel, "run-1");
        let key = DecisionKey::new("credit", "carry", "d1");
        ledger.record(&Spend {
            micro_usd: 5000,
            tokens: 100,
            decision_key: Some(key.clone()),
        });
        ledger.record(&Spend {
            micro_usd: 2000,
            tokens: 50,
            decision_key: Some(key.clone()),
        });
        assert_eq!(ledger.spend_by(&key), 7000);
        assert_eq!(ledger.total_micro_usd(), 7000);
        assert_eq!(sink.len(), 2);
    }

    #[test]
    fn budget_cap_triggers_exceeded() {
        let ledger = SpendLedger::new();
        let key = DecisionKey::new("credit", "carry", "d1");
        ledger.record(&Spend {
            micro_usd: 12_000,
            tokens: 100,
            decision_key: Some(key),
        });
        // desk-scope cap of $0.01 (10_000 micro) -> exceeded
        let b = Budget {
            cap: Cap::Money(10_000),
            scope: BudgetScope::Desk("credit".into()),
        };
        assert!(ledger.check(&b).is_err());
        // global cap higher -> ok
        let ok = Budget {
            cap: Cap::Money(100_000),
            scope: BudgetScope::Global,
        };
        assert!(ledger.check(&ok).is_ok());
    }
}
