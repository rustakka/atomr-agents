//! Agent specs + budgets.
//!
//! `Agent<I, T, Ms, Sk>` is monomorphized over four strategy traits in
//! the Rust crate. Exposing the typed runtime to Python requires a
//! `BoxedAgent` form that doesn't yet exist in `atomr-agents-agent`;
//! this module ships the *static* shape now (AgentSpec, AgentBudgets,
//! TurnResult) so Python config callers can describe an agent.
//!
//! `Agent.run_turn` is exposed via the guest-mode dispatcher in
//! `crate::guest` once the boxed form lands upstream — for now,
//! `AgentSpec` round-trips through the registry and is the unit users
//! manipulate from Python.

use std::time::Duration;

use atomr_agents_agent::{AgentSpec, TurnResult};
use atomr_agents_core::{AgentId, IterationBudget, MoneyBudget, TimeBudget, TokenBudget};
use pyo3::prelude::*;

use crate::core::{PyIterationBudget, PyMoneyBudget, PyTimeBudget, PyTokenBudget, PyTokenUsage};

#[pyclass(name = "AgentSpec", module = "atomr_agents._native.agent")]
#[derive(Clone)]
pub struct PyAgentSpec {
    pub(crate) inner: AgentSpec,
}

#[pymethods]
impl PyAgentSpec {
    #[new]
    #[pyo3(signature = (id, model, max_iterations=8, token_budget=8000, time_budget_ms=60_000, money_budget_usd=1.0))]
    fn new(
        id: String,
        model: String,
        max_iterations: u32,
        token_budget: u32,
        time_budget_ms: u64,
        money_budget_usd: f64,
    ) -> Self {
        Self {
            inner: AgentSpec {
                id: AgentId::from(id),
                model,
                max_iterations,
                token_budget,
                time_budget_ms,
                money_budget_usd,
            },
        }
    }

    #[getter]
    fn id(&self) -> &str {
        self.inner.id.as_str()
    }
    #[getter]
    fn model(&self) -> &str {
        &self.inner.model
    }
    #[getter]
    fn max_iterations(&self) -> u32 {
        self.inner.max_iterations
    }
    #[getter]
    fn token_budget(&self) -> u32 {
        self.inner.token_budget
    }
    #[getter]
    fn time_budget_ms(&self) -> u64 {
        self.inner.time_budget_ms
    }
    #[getter]
    fn money_budget_usd(&self) -> f64 {
        self.inner.money_budget_usd
    }

    /// Materialize the four budgets implied by this spec.
    fn default_budgets(&self) -> PyAgentBudgets {
        let (t, time, m, i) = self.inner.default_budgets();
        PyAgentBudgets {
            tokens: PyTokenBudget { inner: t },
            time: PyTimeBudget { inner: time },
            money: PyMoneyBudget { inner: m },
            iterations: PyIterationBudget { inner: i },
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "AgentSpec(id={:?}, model={:?}, max_iter={}, tokens={})",
            self.inner.id.as_str(),
            self.inner.model,
            self.inner.max_iterations,
            self.inner.token_budget,
        )
    }
}

/// Bundle of the four budgets passed to `Agent.run_turn`.
#[pyclass(name = "AgentBudgets", module = "atomr_agents._native.agent")]
#[derive(Clone)]
pub struct PyAgentBudgets {
    #[pyo3(get)]
    pub tokens: PyTokenBudget,
    #[pyo3(get)]
    pub time: PyTimeBudget,
    #[pyo3(get)]
    pub money: PyMoneyBudget,
    #[pyo3(get)]
    pub iterations: PyIterationBudget,
}

#[pymethods]
impl PyAgentBudgets {
    #[new]
    fn new(
        tokens: PyTokenBudget,
        time: PyTimeBudget,
        money: PyMoneyBudget,
        iterations: PyIterationBudget,
    ) -> Self {
        Self {
            tokens,
            time,
            money,
            iterations,
        }
    }

    #[staticmethod]
    fn defaults() -> Self {
        Self {
            tokens: PyTokenBudget {
                inner: TokenBudget::new(8000),
            },
            time: PyTimeBudget {
                inner: TimeBudget::new(Duration::from_secs(60)),
            },
            money: PyMoneyBudget {
                inner: MoneyBudget::from_usd(1.0),
            },
            iterations: PyIterationBudget {
                inner: IterationBudget::new(8),
            },
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "AgentBudgets(tokens={}, time_ms={}, money_uusd={}, iter={})",
            self.tokens.inner.remaining,
            self.time.inner.remaining_ms,
            self.money.inner.remaining_micro_usd,
            self.iterations.inner.remaining,
        )
    }
}

/// Outcome of a single turn. Mirrors `atomr_agents_agent::TurnResult`.
/// Fields: `text`, `usage`, `finish_reason`, `tool_calls`.
#[pyclass(name = "TurnResult", module = "atomr_agents._native.agent")]
pub struct PyTurnResult {
    pub(crate) inner: TurnResult,
}

#[pymethods]
impl PyTurnResult {
    #[getter]
    fn text(&self) -> &str {
        &self.inner.text
    }

    #[getter]
    fn usage(&self) -> PyTokenUsage {
        PyTokenUsage::from(self.inner.usage)
    }

    #[getter]
    fn finish_reason(&self) -> Option<crate::core::PyFinishReason> {
        self.inner
            .finish_reason
            .map(crate::core::PyFinishReason::from)
    }

    #[getter]
    fn tool_calls(&self) -> Vec<crate::tool::PyParsedToolCall> {
        self.inner
            .tool_calls
            .iter()
            .cloned()
            .map(|inner| crate::tool::PyParsedToolCall { inner })
            .collect()
    }

    fn __repr__(&self) -> String {
        format!(
            "TurnResult(text={:?}, usage={:?}, tool_calls={})",
            self.inner.text,
            self.inner.usage,
            self.inner.tool_calls.len()
        )
    }
}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "agent")?;
    m.add_class::<PyAgentSpec>()?;
    m.add_class::<PyAgentBudgets>()?;
    m.add_class::<PyTurnResult>()?;
    parent.add_submodule(&m)?;
    Ok(())
}
