use serde::{Deserialize, Serialize};

use crate::suite::EvalRun;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionResult {
    pub baseline_pass_rate: f32,
    pub current_pass_rate: f32,
    pub delta: f32,
    pub blocked: bool,
    pub reason: String,
}

/// Compare a current `EvalRun` against a baseline. Blocks publication
/// if pass-rate regressed by more than `tolerance`.
pub struct RegressionGate {
    pub tolerance: f32,
}

impl RegressionGate {
    pub fn check(&self, baseline: &EvalRun, current: &EvalRun) -> RegressionResult {
        let delta = current.pass_rate() - baseline.pass_rate();
        let blocked = delta < -self.tolerance;
        let reason = if blocked {
            format!(
                "pass_rate dropped from {:.2} to {:.2} (tolerance {:.2})",
                baseline.pass_rate(),
                current.pass_rate(),
                self.tolerance
            )
        } else {
            "ok".into()
        };
        RegressionResult {
            baseline_pass_rate: baseline.pass_rate(),
            current_pass_rate: current.pass_rate(),
            delta,
            blocked,
            reason,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_with(passed: u32, failed: u32) -> EvalRun {
        EvalRun {
            passed,
            failed,
            avg_score: 0.0,
            results: vec![],
        }
    }

    #[test]
    fn regression_blocks_when_below_tolerance() {
        let gate = RegressionGate { tolerance: 0.05 };
        let baseline = run_with(9, 1); // 0.9
        let current = run_with(7, 3); // 0.7
        let r = gate.check(&baseline, &current);
        assert!(r.blocked);
    }

    #[test]
    fn regression_allows_within_tolerance() {
        let gate = RegressionGate { tolerance: 0.10 };
        let baseline = run_with(10, 0); // 1.0
        let current = run_with(95, 5); // 0.95
        let r = gate.check(&baseline, &current);
        assert!(!r.blocked, "delta {} blocked unexpectedly", r.delta);
    }
}
