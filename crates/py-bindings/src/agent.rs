//! Agent specs + budgets + runtime (via `BoxedAgent`).
//!
//! The generic `Agent<I, T, Ms, Sk>` is type-erased through the
//! `BoxedAgent` wrapper (see `atomr-agents-agent::boxed`). Python
//! constructs an agent by handing strategy dyn handles to
//! `AgentBuilder`, which produces a `PyAgentRef` (callable + runnable).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use atomr_agents_agent::{
    AgentSpec, BoxedAgent, InferenceClient, LoggingMiddleware, RateLimitMiddleware,
    RedactionMiddleware, ToolErrorRecoveryMiddleware, TurnResult,
};
use atomr_agents_core::{
    AgentError, AgentId, IterationBudget, MoneyBudget, Result as AgentResult, TimeBudget,
    TokenBudget,
};
use atomr_agents_tool::Provider;
use atomr_infer_core::batch::ExecuteBatch;
use pyo3::prelude::*;

use crate::core::{PyIterationBudget, PyMoneyBudget, PyTimeBudget, PyTokenBudget, PyTokenUsage};
use crate::strategy::await_if_coro;

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

// ----- Inference client + adapter -----------------------------------------

/// Python handle on a Rust `InferenceClient`. Construct via
/// `inference_client_from_factory` (Python guest implementing
/// `provider() -> str` and `async run(batch) -> {text, usage, ...}`),
/// or via `mock_inference_client(...)` for deterministic tests.
#[pyclass(name = "InferenceClient", module = "atomr_agents._native.agent")]
#[derive(Clone)]
pub struct PyInferenceClient {
    pub(crate) inner: Arc<dyn InferenceClient>,
}

#[pymethods]
impl PyInferenceClient {
    fn provider(&self) -> String {
        match self.inner.provider() {
            Provider::Anthropic => "anthropic".into(),
            Provider::OpenAi => "openai".into(),
        }
    }

    fn __repr__(&self) -> String {
        format!("InferenceClient(provider={})", self.provider())
    }
}

pub(crate) struct PyInferenceClientAdapter {
    target: Arc<PyObject>,
    provider: Provider,
}

#[async_trait]
impl InferenceClient for PyInferenceClientAdapter {
    fn provider(&self) -> Provider {
        self.provider
    }

    async fn run(&self, batch: ExecuteBatch) -> AgentResult<TurnResult> {
        // Convert ExecuteBatch to a Python dict-like representation,
        // then await the Python side's `run`. Return a TurnResult
        // mapped from a dict shape `{text, input_tokens, output_tokens,
        // finish_reason, tool_calls}`.
        let target = self.target.clone();
        let batch_json = serde_json::json!({
            "request_id": batch.request_id,
            "model": batch.model,
            "estimated_tokens": batch.estimated_tokens,
            "stream": batch.stream,
            "messages": batch.messages.iter().map(|m| {
                let content = match &m.content {
                    atomr_infer_core::batch::MessageContent::Text(s) => s.clone(),
                    _ => String::new(),
                };
                let role = match m.role {
                    atomr_infer_core::batch::Role::System => "system",
                    atomr_infer_core::batch::Role::User => "user",
                    atomr_infer_core::batch::Role::Assistant => "assistant",
                    atomr_infer_core::batch::Role::Tool => "tool",
                    _ => "user",
                };
                serde_json::json!({"role": role, "content": content})
            }).collect::<Vec<_>>(),
        });
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let arg = crate::conv::json_to_py(py, &batch_json)?;
            let instance: Bound<'_, PyAny> = if bound.hasattr("run")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("run")?.call1((arg,))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Inference(format!("py inference client: {e}")))?;
        let final_val = await_if_coro(coro_or_val).await?;
        Python::with_gil(|py| -> PyResult<TurnResult> {
            let bound = final_val.bind(py);
            let text: String = bound
                .get_item("text")
                .and_then(|v| v.extract())
                .unwrap_or_default();
            let input_tokens: u32 = bound
                .get_item("input_tokens")
                .and_then(|v| v.extract())
                .unwrap_or(0);
            let output_tokens: u32 = bound
                .get_item("output_tokens")
                .and_then(|v| v.extract())
                .unwrap_or(0);
            Ok(TurnResult {
                text,
                usage: atomr_infer_core::tokens::TokenUsage {
                    input_tokens,
                    output_tokens,
                    reasoning_tokens: 0,
                    cached_tokens: 0,
                },
                finish_reason: None,
                tool_calls: Vec::new(),
            })
        })
        .map_err(|e| AgentError::Inference(format!("py inference client result: {e}")))
    }
}

#[pyfunction]
#[pyo3(signature = (key, provider="openai"))]
fn inference_client_from_factory(key: String, provider: &str) -> PyResult<PyInferenceClient> {
    let target = crate::guest::must_lookup("inference_client", &key)?;
    let provider = match provider {
        "anthropic" => Provider::Anthropic,
        _ => Provider::OpenAi,
    };
    Ok(PyInferenceClient {
        inner: Arc::new(PyInferenceClientAdapter { target, provider }),
    })
}

// ----- Middleware ---------------------------------------------------------

#[pyclass(name = "AgentMiddleware", module = "atomr_agents._native.agent")]
#[derive(Clone)]
pub struct PyAgentMiddleware {
    pub(crate) inner: Arc<dyn atomr_agents_agent::AgentMiddleware>,
}

#[pymethods]
impl PyAgentMiddleware {
    fn __repr__(&self) -> String {
        "AgentMiddleware(handle)".into()
    }
}

#[pyfunction]
fn logging_middleware() -> PyAgentMiddleware {
    PyAgentMiddleware {
        inner: Arc::new(LoggingMiddleware::new()),
    }
}

#[pyfunction]
fn tool_error_recovery_middleware() -> PyAgentMiddleware {
    PyAgentMiddleware {
        inner: Arc::new(ToolErrorRecoveryMiddleware),
    }
}

#[pyfunction]
#[pyo3(signature = (patterns, replacement="[redacted]".to_string()))]
fn redaction_middleware(patterns: Vec<String>, replacement: String) -> PyAgentMiddleware {
    PyAgentMiddleware {
        inner: Arc::new(RedactionMiddleware::new(patterns, replacement)),
    }
}

#[pyfunction]
fn rate_limit_middleware(capacity: u32, refill_per_sec: u32) -> PyAgentMiddleware {
    PyAgentMiddleware {
        inner: Arc::new(RateLimitMiddleware::new(capacity, refill_per_sec)),
    }
}

// ----- AgentBuilder + AgentRef --------------------------------------------

/// Builder over the four strategy slots + inference client + event
/// bus. Produces `PyAgentRef` (a callable + runnable agent).
#[pyclass(name = "AgentBuilder", module = "atomr_agents._native.agent")]
pub struct PyAgentBuilder {
    id: AgentId,
    model: String,
    instructions: Option<crate::instruction::PyInstructionStrategy>,
    tools: Option<crate::strategy::PyToolStrategy>,
    memory: Option<crate::strategy::PyMemoryStrategy>,
    skills: Option<crate::strategy::PySkillStrategy>,
    inference: Option<PyInferenceClient>,
    bus: Option<crate::observability::PyEventBus>,
    max_tool_iterations: u32,
}

#[pymethods]
impl PyAgentBuilder {
    #[new]
    #[pyo3(signature = (id, model, max_tool_iterations=8))]
    fn new(id: String, model: String, max_tool_iterations: u32) -> Self {
        Self {
            id: AgentId::from(id),
            model,
            instructions: None,
            tools: None,
            memory: None,
            skills: None,
            inference: None,
            bus: None,
            max_tool_iterations,
        }
    }

    fn with_instructions(mut slf: PyRefMut<'_, Self>, instructions: crate::instruction::PyInstructionStrategy) {
        slf.instructions = Some(instructions);
    }
    fn with_tools(mut slf: PyRefMut<'_, Self>, tools: crate::strategy::PyToolStrategy) {
        slf.tools = Some(tools);
    }
    fn with_memory(mut slf: PyRefMut<'_, Self>, memory: crate::strategy::PyMemoryStrategy) {
        slf.memory = Some(memory);
    }
    fn with_skills(mut slf: PyRefMut<'_, Self>, skills: crate::strategy::PySkillStrategy) {
        slf.skills = Some(skills);
    }
    fn with_inference(mut slf: PyRefMut<'_, Self>, inference: PyInferenceClient) {
        slf.inference = Some(inference);
    }
    fn with_event_bus(mut slf: PyRefMut<'_, Self>, bus: crate::observability::PyEventBus) {
        slf.bus = Some(bus);
    }

    fn build(slf: PyRef<'_, Self>) -> PyResult<PyAgentRef> {
        // Adapt Arc<dyn TraitX> to Box<dyn TraitX> for BoxedAgent.
        let instructions = slf
            .instructions
            .clone()
            .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("instructions strategy required"))?;
        let tools = slf
            .tools
            .clone()
            .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("tool strategy required"))?;
        let memory = slf
            .memory
            .clone()
            .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("memory strategy required"))?;
        let skills = slf
            .skills
            .clone()
            .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("skill strategy required"))?;
        let inference = slf
            .inference
            .clone()
            .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("inference client required"))?;
        let bus = slf
            .bus
            .clone()
            .unwrap_or_else(|| crate::observability::PyEventBus::new_default());

        // Wrap Arcs in newtype Boxes that implement the trait by delegating.
        let agent = BoxedAgent::new(
            slf.id.clone(),
            slf.model.clone(),
            Box::new(ArcInstr(instructions.inner)),
            Box::new(ArcTools(tools.inner)),
            Box::new(ArcMem(memory.inner)),
            Box::new(ArcSkill(skills.inner)),
            inference.inner.clone(),
            bus.inner.clone(),
            slf.max_tool_iterations,
        );
        let ref_ = agent.into_ref();
        Ok(PyAgentRef {
            inner: Arc::new(ref_),
        })
    }
}

// Newtype wrappers so `Arc<dyn>` strategies fit in `Box<dyn>` slots.
struct ArcInstr(Arc<dyn atomr_agents_instruction::InstructionStrategy>);
#[async_trait]
impl atomr_agents_instruction::InstructionStrategy for ArcInstr {
    async fn render(
        &self,
        ctx: &atomr_agents_core::AgentContext,
        budget: &mut TokenBudget,
    ) -> AgentResult<atomr_agents_instruction::RenderedInstructions> {
        self.0.render(ctx, budget).await
    }
}

struct ArcTools(Arc<dyn atomr_agents_strategy::ToolStrategy>);
#[async_trait]
impl atomr_agents_strategy::ToolStrategy for ArcTools {
    async fn select(
        &self,
        ctx: &atomr_agents_core::AgentContext,
        budget: &mut TokenBudget,
    ) -> AgentResult<Vec<atomr_agents_strategy::ToolRef>> {
        self.0.select(ctx, budget).await
    }
}

struct ArcMem(Arc<dyn atomr_agents_strategy::MemoryStrategy>);
#[async_trait]
impl atomr_agents_strategy::MemoryStrategy for ArcMem {
    async fn retrieve(
        &self,
        ctx: &atomr_agents_core::AgentContext,
        budget: &mut TokenBudget,
    ) -> AgentResult<Vec<atomr_agents_core::MemoryChunk>> {
        self.0.retrieve(ctx, budget).await
    }
    async fn store(&self, item: atomr_agents_core::MemoryItem) -> AgentResult<()> {
        self.0.store(item).await
    }
}

struct ArcSkill(Arc<dyn atomr_agents_strategy::SkillStrategy>);
#[async_trait]
impl atomr_agents_strategy::SkillStrategy for ArcSkill {
    async fn applicable(
        &self,
        ctx: &atomr_agents_core::AgentContext,
        budget: &mut TokenBudget,
    ) -> AgentResult<Vec<atomr_agents_strategy::SkillRef>> {
        self.0.applicable(ctx, budget).await
    }
}

/// Callable + runnable agent handle. `as_callable()` exposes it as a
/// `PyCallable`; `run_turn(user, budgets)` runs one turn directly.
#[pyclass(name = "AgentRef", module = "atomr_agents._native.agent")]
#[derive(Clone)]
pub struct PyAgentRef {
    pub(crate) inner: Arc<atomr_agents_agent::AgentRef>,
}

#[pymethods]
impl PyAgentRef {
    fn run_turn<'py>(
        &self,
        py: Python<'py>,
        user: String,
        budgets: Option<PyAgentBudgets>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let budgets = budgets.unwrap_or_else(PyAgentBudgets::defaults);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let ctx = atomr_agents_core::CallCtx {
                agent_id: Some(atomr_agents_core::AgentId::from(inner.id.as_str().to_string())),
                tokens: budgets.tokens.inner,
                time: budgets.time.inner,
                money: budgets.money.inner,
                iterations: budgets.iterations.inner,
                trace: Vec::new(),
            };
            let r = inner.turn(user, ctx).await.map_err(crate::errors::map)?;
            Ok(PyTurnResult { inner: r })
        })
    }

    fn as_callable(&self) -> crate::callable::PyCallable {
        let inner: atomr_agents_callable::CallableHandle = self.inner.clone();
        crate::callable::PyCallable::from_handle(inner)
    }

    #[getter]
    fn id(&self) -> &str {
        self.inner.id.as_str()
    }

    fn __repr__(&self) -> String {
        format!("AgentRef(id={:?})", self.inner.id.as_str())
    }
}

// We need a helper on PyEventBus for the default-constructed case.
// PyEventBus is in observability.rs — add a free function here that
// asks observability to make one. Provide via PyEventBus::new_default.

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "agent")?;
    m.add_class::<PyAgentSpec>()?;
    m.add_class::<PyAgentBudgets>()?;
    m.add_class::<PyTurnResult>()?;
    m.add_class::<PyInferenceClient>()?;
    m.add_class::<PyAgentMiddleware>()?;
    m.add_class::<PyAgentBuilder>()?;
    m.add_class::<PyAgentRef>()?;
    m.add_function(wrap_pyfunction!(inference_client_from_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(logging_middleware, &m)?)?;
    m.add_function(wrap_pyfunction!(tool_error_recovery_middleware, &m)?)?;
    m.add_function(wrap_pyfunction!(redaction_middleware, &m)?)?;
    m.add_function(wrap_pyfunction!(rate_limit_middleware, &m)?)?;
    parent.add_submodule(&m)?;
    Ok(())
}
