//! Agent specs + budgets + the `PyAgent` runtime handle.
//!
//! `AgentSpec` / `AgentBudgets` / `TurnResult` are the static config
//! shapes Python users describe their agents with. `PyAgent` is the
//! runnable handle: built via `Agent.from_spec(...)` against four
//! strategies looked up in the guest registry plus an inference
//! provider key, and driven via the async `run_turn(user)` coroutine.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use atomr_agents_agent::{AgentSpec, InferenceClient, TurnResult};
use atomr_agents_core::{
    AgentContext, AgentError, AgentId, CallCtx, IterationBudget, MemoryChunk, MemoryItem, MoneyBudget,
    Result as AgentResult, TimeBudget, TokenBudget,
};
use atomr_agents_instruction::{InstructionStrategy, RenderedInstructions};
use atomr_agents_strategy::{MemoryStrategy, SkillRef, SkillStrategy, ToolStrategy};
use atomr_agents_tool::{StaticToolStrategy, ToolSet};
use pyo3::prelude::*;

use crate::core::{PyIterationBudget, PyMoneyBudget, PyTimeBudget, PyTokenBudget, PyTokenUsage};
use crate::guest::{
    build_guest_instruction_strategy, build_guest_memory_strategy, build_guest_skill_strategy,
    build_guest_toolset, PyPersona,
};

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
        self.inner.finish_reason.map(crate::core::PyFinishReason::from)
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

// ---------------------------------------------------------------------------
// PyAgent — runnable agent handle.
// ---------------------------------------------------------------------------
//
// Strategy handle types (`PyInstruction`, `PyMemoryStrategyHandle`, ...)
// hold their adapters as `Arc<dyn Trait>`, but `AgentSpec::into_agent`
// expects `Box<dyn Trait>`. The arc-forwarding wrappers below let us
// move a handle's `Arc<dyn ...>` into the boxed slot without rebuilding
// the adapter from the registered Python target.

struct ArcInstructionStrategy(Arc<dyn InstructionStrategy>);
#[async_trait]
impl InstructionStrategy for ArcInstructionStrategy {
    async fn render(
        &self,
        ctx: &AgentContext,
        budget: &mut TokenBudget,
    ) -> AgentResult<RenderedInstructions> {
        self.0.render(ctx, budget).await
    }
}

struct ArcMemoryStrategy(Arc<dyn MemoryStrategy>);
#[async_trait]
impl MemoryStrategy for ArcMemoryStrategy {
    async fn retrieve(&self, ctx: &AgentContext, budget: &mut TokenBudget) -> AgentResult<Vec<MemoryChunk>> {
        self.0.retrieve(ctx, budget).await
    }

    async fn store(&self, item: MemoryItem) -> AgentResult<()> {
        self.0.store(item).await
    }
}

struct ArcSkillStrategy(Arc<dyn SkillStrategy>);
#[async_trait]
impl SkillStrategy for ArcSkillStrategy {
    async fn applicable(&self, ctx: &AgentContext, budget: &mut TokenBudget) -> AgentResult<Vec<SkillRef>> {
        self.0.applicable(ctx, budget).await
    }
}

/// `ToolSet` is a versioned bundle of `DynTool` — feed its tool list
/// into the canonical "fixed list" `StaticToolStrategy` so the agent
/// pipeline sees them per turn.
fn toolset_to_strategy(tools: Arc<ToolSet>) -> Box<dyn ToolStrategy> {
    Box::new(StaticToolStrategy::new(tools.tools.clone()))
}

/// Runnable agent. Build via [`PyAgent::from_spec`]; drive with the
/// async [`PyAgent::run_turn`] coroutine.
#[pyclass(name = "Agent", module = "atomr_agents._native.agent")]
pub struct PyAgent {
    pub(crate) inner: Arc<atomr_agents_agent::AgentRef>,
    pub(crate) id: AgentId,
}

#[pymethods]
impl PyAgent {
    /// Build a runnable agent from a spec, four strategy keys, an
    /// optional persona key (`""` → default static persona), an
    /// optional list of tool keys (omit / empty → empty toolset), and
    /// an inference provider name. Strategies are resolved via the
    /// guest registry; tools are resolved via `register_tool_factory`
    /// entries with descriptors.
    ///
    /// `inference_provider` is one of `"mock" | "anthropic" | "openai"
    /// | "gemini"`. Only `"mock"` is wired in this build — the real
    /// providers require `provider-*` features on
    /// `atomr-agents-agent`. See `crate::inference` for details.
    #[staticmethod]
    #[pyo3(signature = (
        spec,
        instruction_key,
        memory_key,
        skill_key,
        persona_key = String::new(),
        tool_keys = None,
        inference_provider = String::from("mock"),
    ))]
    fn from_spec(
        spec: PyAgentSpec,
        instruction_key: String,
        memory_key: String,
        skill_key: String,
        persona_key: String,
        tool_keys: Option<Vec<String>>,
        inference_provider: String,
    ) -> PyResult<Self> {
        // 1. Strategies (instruction / memory / skill) — each guest
        //    handle holds an `Arc<dyn Trait>` constructed from the
        //    registered Python target; wrap into `Box<dyn Trait>` for
        //    `AgentSpec::into_agent`.
        let inst_handle = build_guest_instruction_strategy(instruction_key)?;
        let mem_handle = build_guest_memory_strategy(memory_key)?;
        let skill_handle = build_guest_skill_strategy(skill_key)?;

        let instructions: Box<dyn InstructionStrategy> = Box::new(ArcInstructionStrategy(inst_handle.inner));
        let memory: Box<dyn MemoryStrategy> = Box::new(ArcMemoryStrategy(mem_handle.inner));
        let skills: Box<dyn SkillStrategy> = Box::new(ArcSkillStrategy(skill_handle.inner));

        // 2. Tools: an empty list (or unspecified) yields an empty
        //    `StaticToolStrategy`; otherwise build a `ToolSet` from
        //    the requested keys via the existing guest helper.
        let tool_keys = tool_keys.unwrap_or_default();
        let tools: Box<dyn ToolStrategy> = if tool_keys.is_empty() {
            Box::new(StaticToolStrategy::new(Vec::new()))
        } else {
            // Reuse the registry-driven toolset builder. Use the spec
            // id + a "0.0.0" version since the toolset id only matters
            // for downstream artifact tracking and a per-spec value is
            // descriptive enough.
            let toolset = build_guest_toolset(
                format!("agent:{}", spec.inner.id.as_str()),
                "0.0.0",
                Some(tool_keys),
            )?;
            toolset_to_strategy(toolset.inner)
        };

        // 3. Persona (optional). The guest handle isn't used directly
        //    here — `AgentSpec::into_agent` doesn't take a persona
        //    slot; persona is normally composed inside
        //    `InstructionStrategy`. We therefore only validate that
        //    the registered persona exists when a key is provided, so
        //    misconfiguration surfaces early.
        if !persona_key.is_empty() {
            let _persona: PyPersona = crate::guest::build_guest_persona(persona_key)?;
        }

        // 4. Inference client.
        let inference: Arc<dyn InferenceClient> =
            crate::inference::build_inference_client(&inference_provider)
                .map_err(|e| PyErr::new::<crate::errors::AgentError, _>(e.to_string()))?;

        // 5. Materialize the agent. Holding `Arc<AgentRef>` lets
        //    `run_turn` clone for the spawned future without changing
        //    the upstream `AgentRef` ownership story.
        let id = spec.inner.id.clone();
        let agent_ref = spec
            .inner
            .into_agent(instructions, tools, memory, skills, inference);
        Ok(Self {
            inner: Arc::new(agent_ref),
            id,
        })
    }

    /// Run one agent turn. Returns a Python awaitable that resolves to
    /// a `TurnResult`. Default per-turn budgets are taken from the
    /// originating spec via `AgentBudgets::defaults()`-equivalent
    /// values so a smoke caller doesn't have to thread budgets in.
    fn run_turn<'py>(&self, py: Python<'py>, user: String) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let agent_id = self.id.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let ctx = CallCtx {
                agent_id: Some(agent_id),
                tokens: TokenBudget::new(8_000),
                time: TimeBudget::new(Duration::from_secs(60)),
                money: MoneyBudget::from_usd(1.0),
                iterations: IterationBudget::new(8),
                trace: vec![],
            };
            let result = inner
                .turn(user, ctx)
                .await
                .map_err(|e: AgentError| PyErr::new::<crate::errors::AgentError, _>(e.to_string()))?;
            Python::with_gil(|py| Py::new(py, PyTurnResult { inner: result }).map(|p| p.into_any()))
        })
    }

    #[getter]
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn __repr__(&self) -> String {
        format!("Agent(id={:?})", self.id.as_str())
    }
}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "agent")?;
    m.add_class::<PyAgentSpec>()?;
    m.add_class::<PyAgentBudgets>()?;
    m.add_class::<PyTurnResult>()?;
    m.add_class::<PyAgent>()?;
    parent.add_submodule(&m)?;
    Ok(())
}
