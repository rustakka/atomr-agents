//! Instruction strategies + prompt templates.
//!
//! Concrete templates (`ChatPromptTemplate`, `FewShotChatTemplate`,
//! selectors) are surfaced as `#[pyclass]`es; trait strategies
//! (`InstructionStrategy`, `TaskStrategy`, `BehaviorStrategy`) come in
//! as dyn handles with optional Python guest adapters.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentContext, AgentError, MessageRole, Result as AgentResult, TokenBudget, Value};
use atomr_agents_embed::Embedder;
use atomr_agents_instruction::{
    BehaviorStrategy, ChatPromptTemplate, Example, ExampleSelector, FewShotChatTemplate, InstructionStrategy,
    LengthBasedSelector, MessageTemplate, RenderedInstructions, RenderedMessage, SemanticSimilaritySelector,
    StaticBehaviorStrategy, StaticTaskStrategy, StringTemplate, TaskStrategy,
};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::conv::{json_to_py, py_to_json};
use crate::strategy::{agent_context_to_pydict, await_if_coro};

// ----- RenderedInstructions -----------------------------------------------

#[pyclass(name = "RenderedInstructions", module = "atomr_agents._native.instruction")]
#[derive(Clone)]
pub struct PyRenderedInstructions {
    pub(crate) inner: RenderedInstructions,
}

#[pymethods]
impl PyRenderedInstructions {
    #[new]
    fn new(system_prompt: String, estimated_tokens: u32) -> Self {
        Self {
            inner: RenderedInstructions {
                system_prompt,
                estimated_tokens,
            },
        }
    }

    #[getter]
    fn system_prompt(&self) -> &str {
        &self.inner.system_prompt
    }

    #[getter]
    fn estimated_tokens(&self) -> u32 {
        self.inner.estimated_tokens
    }

    fn __repr__(&self) -> String {
        format!(
            "RenderedInstructions(estimated_tokens={}, chars={})",
            self.inner.estimated_tokens,
            self.inner.system_prompt.chars().count()
        )
    }
}

// ----- RenderedMessage -----------------------------------------------------

#[pyclass(name = "RenderedMessage", module = "atomr_agents._native.instruction")]
#[derive(Clone)]
pub struct PyRenderedMessage {
    pub(crate) inner: RenderedMessage,
}

#[pymethods]
impl PyRenderedMessage {
    #[new]
    fn new(role: &str, content: String) -> Self {
        let role = match role {
            "system" => MessageRole::System,
            "assistant" => MessageRole::Assistant,
            "tool" => MessageRole::Tool,
            _ => MessageRole::User,
        };
        Self {
            inner: RenderedMessage { role, content },
        }
    }

    #[getter]
    fn role(&self) -> &'static str {
        match self.inner.role {
            MessageRole::System => "system",
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::Tool => "tool",
        }
    }

    #[getter]
    fn content(&self) -> &str {
        &self.inner.content
    }

    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new_bound(py);
        d.set_item("role", self.role())?;
        d.set_item("content", &self.inner.content)?;
        Ok(d)
    }

    fn __repr__(&self) -> String {
        format!(
            "RenderedMessage(role={:?}, chars={})",
            self.role(),
            self.inner.content.chars().count()
        )
    }
}

// ----- StringTemplate -----------------------------------------------------

#[pyclass(name = "StringTemplate", module = "atomr_agents._native.instruction")]
#[derive(Clone)]
pub struct PyStringTemplate {
    pub(crate) inner: StringTemplate,
}

#[pymethods]
impl PyStringTemplate {
    #[new]
    fn new(text: String) -> Self {
        Self {
            inner: StringTemplate(text),
        }
    }

    fn render(&self, py: Python<'_>, vars: &Bound<'_, PyDict>) -> PyResult<String> {
        let v = py_to_json(py, vars.as_any())?;
        let map: HashMap<String, Value> = match v {
            Value::Object(m) => m.into_iter().collect(),
            _ => HashMap::new(),
        };
        Ok(self.inner.render(&map))
    }

    fn __repr__(&self) -> String {
        format!("StringTemplate({:?})", self.inner.0)
    }
}

// ----- MessageTemplate ----------------------------------------------------

#[pyclass(name = "MessageTemplate", module = "atomr_agents._native.instruction")]
#[derive(Clone)]
pub struct PyMessageTemplate {
    pub(crate) inner: MessageTemplate,
}

#[pymethods]
impl PyMessageTemplate {
    #[staticmethod]
    fn system(text: String) -> Self {
        Self {
            inner: MessageTemplate::System(StringTemplate(text)),
        }
    }
    #[staticmethod]
    fn user(text: String) -> Self {
        Self {
            inner: MessageTemplate::User(StringTemplate(text)),
        }
    }
    #[staticmethod]
    fn assistant(text: String) -> Self {
        Self {
            inner: MessageTemplate::Assistant(StringTemplate(text)),
        }
    }
    #[staticmethod]
    fn placeholder(key: String) -> Self {
        Self {
            inner: MessageTemplate::Placeholder { key },
        }
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            MessageTemplate::System(t) => format!("MessageTemplate(system, {:?})", t.0),
            MessageTemplate::User(t) => format!("MessageTemplate(user, {:?})", t.0),
            MessageTemplate::Assistant(t) => format!("MessageTemplate(assistant, {:?})", t.0),
            MessageTemplate::Placeholder { key } => {
                format!("MessageTemplate(placeholder, key={key:?})")
            }
        }
    }
}

// ----- ChatPromptTemplate (+ Builder) -------------------------------------

#[pyclass(name = "ChatPromptTemplate", module = "atomr_agents._native.instruction")]
#[derive(Clone)]
pub struct PyChatPromptTemplate {
    pub(crate) inner: ChatPromptTemplate,
}

#[pymethods]
impl PyChatPromptTemplate {
    #[new]
    fn new() -> Self {
        Self {
            inner: ChatPromptTemplate::builder().build(),
        }
    }

    #[staticmethod]
    fn from_messages(messages: Vec<PyMessageTemplate>) -> Self {
        Self {
            inner: ChatPromptTemplate {
                messages: messages.into_iter().map(|m| m.inner).collect(),
                partial: HashMap::new(),
            },
        }
    }

    fn with_partial(&self, py: Python<'_>, key: String, val: &Bound<'_, PyAny>) -> PyResult<Self> {
        let v = py_to_json(py, val)?;
        Ok(Self {
            inner: self.inner.clone().partial(key, v),
        })
    }

    fn render<'py>(&self, py: Python<'py>, vars: &Bound<'py, PyDict>) -> PyResult<Vec<PyRenderedMessage>> {
        let v = py_to_json(py, vars.as_any())?;
        let map: HashMap<String, Value> = match v {
            Value::Object(m) => m.into_iter().collect(),
            _ => HashMap::new(),
        };
        let rendered = self.inner.render(&map).map_err(crate::errors::map)?;
        Ok(rendered
            .into_iter()
            .map(|inner| PyRenderedMessage { inner })
            .collect())
    }

    fn __repr__(&self) -> String {
        format!("ChatPromptTemplate(messages={})", self.inner.messages.len())
    }
}

/// Builder factory pattern: `builder().system(...).user(...).build()`.
#[pyclass(
    name = "ChatPromptTemplateBuilder",
    module = "atomr_agents._native.instruction"
)]
pub struct PyChatPromptTemplateBuilder {
    messages: Vec<MessageTemplate>,
    partial: HashMap<String, Value>,
}

#[pymethods]
impl PyChatPromptTemplateBuilder {
    #[new]
    fn new() -> Self {
        Self {
            messages: Vec::new(),
            partial: HashMap::new(),
        }
    }

    fn system(&mut self, text: String) -> PyResult<()> {
        self.messages.push(MessageTemplate::System(StringTemplate(text)));
        Ok(())
    }

    fn user(&mut self, text: String) -> PyResult<()> {
        self.messages.push(MessageTemplate::User(StringTemplate(text)));
        Ok(())
    }

    fn assistant(&mut self, text: String) -> PyResult<()> {
        self.messages
            .push(MessageTemplate::Assistant(StringTemplate(text)));
        Ok(())
    }

    fn placeholder(&mut self, key: String) -> PyResult<()> {
        self.messages.push(MessageTemplate::Placeholder { key });
        Ok(())
    }

    fn partial(&mut self, py: Python<'_>, key: String, val: &Bound<'_, PyAny>) -> PyResult<()> {
        let v = py_to_json(py, val)?;
        self.partial.insert(key, v);
        Ok(())
    }

    fn build(&mut self) -> PyChatPromptTemplate {
        let messages = std::mem::take(&mut self.messages);
        let partial = std::mem::take(&mut self.partial);
        PyChatPromptTemplate {
            inner: ChatPromptTemplate { messages, partial },
        }
    }
}

// ----- Example + ExampleSelector ------------------------------------------

#[pyclass(name = "Example", module = "atomr_agents._native.instruction")]
#[derive(Clone)]
pub struct PyExample {
    pub(crate) inner: Example,
}

#[pymethods]
impl PyExample {
    #[new]
    #[pyo3(signature = (vars=None, estimated_tokens=0, query_text=String::new()))]
    fn new(
        py: Python<'_>,
        vars: Option<&Bound<'_, PyDict>>,
        estimated_tokens: u32,
        query_text: String,
    ) -> PyResult<Self> {
        let map: HashMap<String, Value> = match vars {
            Some(d) => {
                let v = py_to_json(py, d.as_any())?;
                match v {
                    Value::Object(m) => m.into_iter().collect(),
                    _ => HashMap::new(),
                }
            }
            None => HashMap::new(),
        };
        Ok(Self {
            inner: Example {
                vars: map,
                estimated_tokens,
                query_text,
            },
        })
    }

    #[getter]
    fn estimated_tokens(&self) -> u32 {
        self.inner.estimated_tokens
    }

    #[getter]
    fn query_text(&self) -> &str {
        &self.inner.query_text
    }

    #[getter]
    fn vars(&self, py: Python<'_>) -> PyResult<PyObject> {
        let obj = serde_json::Value::Object(self.inner.vars.clone().into_iter().collect());
        json_to_py(py, &obj)
    }

    fn __repr__(&self) -> String {
        format!(
            "Example(query={:?}, estimated_tokens={})",
            self.inner.query_text, self.inner.estimated_tokens
        )
    }
}

#[pyclass(name = "ExampleSelector", module = "atomr_agents._native.instruction")]
#[derive(Clone)]
pub struct PyExampleSelector {
    pub(crate) inner: Arc<dyn ExampleSelector>,
}

#[pymethods]
impl PyExampleSelector {
    fn __repr__(&self) -> String {
        "ExampleSelector(handle)".into()
    }
}

#[pyfunction]
#[pyo3(signature = (examples, max_tokens))]
fn length_based_selector(examples: Vec<PyExample>, max_tokens: u32) -> PyExampleSelector {
    PyExampleSelector {
        inner: Arc::new(LengthBasedSelector {
            examples: examples.into_iter().map(|e| e.inner).collect(),
            max_tokens,
        }),
    }
}

#[pyfunction]
#[pyo3(signature = (examples, embedder, query_key, top_k))]
fn semantic_similarity_selector(
    examples: Vec<PyExample>,
    embedder: PyObject,
    query_key: String,
    top_k: usize,
) -> PyResult<PyExampleSelector> {
    let embedder_handle = Python::with_gil(|py| -> PyResult<Arc<dyn Embedder>> {
        let bound = embedder.bind(py);
        // Allow either a guest factory key (string) or a PyEmbedder handle.
        if let Ok(s) = bound.extract::<String>() {
            let target = crate::guest::must_lookup("embedder", &s)?;
            Ok(Arc::new(crate::embed::PyEmbedderAdapter { target }) as Arc<dyn Embedder>)
        } else {
            let h: crate::embed::PyEmbedder = bound.extract()?;
            Ok(h.inner)
        }
    })?;
    Ok(PyExampleSelector {
        inner: Arc::new(SemanticSimilaritySelector {
            examples: examples.into_iter().map(|e| e.inner).collect(),
            embedder: embedder_handle,
            query_key,
            top_k,
        }),
    })
}

// ----- FewShotChatTemplate ------------------------------------------------

#[pyclass(name = "FewShotChatTemplate", module = "atomr_agents._native.instruction")]
#[derive(Clone)]
pub struct PyFewShotChatTemplate {
    pub(crate) inner: Arc<FewShotChatTemplate>,
}

#[pymethods]
impl PyFewShotChatTemplate {
    #[new]
    fn new(
        formatter: PyChatPromptTemplate,
        selector: PyExampleSelector,
        example_template: PyChatPromptTemplate,
    ) -> Self {
        Self {
            inner: Arc::new(FewShotChatTemplate::new(
                formatter.inner,
                selector.inner,
                example_template.inner,
            )),
        }
    }

    fn render<'py>(&self, py: Python<'py>, vars: &Bound<'py, PyDict>) -> PyResult<Bound<'py, PyAny>> {
        let v = py_to_json(py, vars.as_any())?;
        let map: HashMap<String, Value> = match v {
            Value::Object(m) => m.into_iter().collect(),
            _ => HashMap::new(),
        };
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let rendered = inner.render(&map).await.map_err(crate::errors::map)?;
            Python::with_gil(|py| -> PyResult<PyObject> {
                let list = PyList::empty_bound(py);
                for m in rendered {
                    let row = PyDict::new_bound(py);
                    let role = match m.role {
                        MessageRole::System => "system",
                        MessageRole::User => "user",
                        MessageRole::Assistant => "assistant",
                        MessageRole::Tool => "tool",
                    };
                    row.set_item("role", role)?;
                    row.set_item("content", m.content)?;
                    list.append(row)?;
                }
                Ok(list.unbind().into())
            })
        })
    }

    fn __repr__(&self) -> String {
        "FewShotChatTemplate".into()
    }
}

// ----- InstructionStrategy (dyn handle + adapter) -------------------------

#[pyclass(name = "InstructionStrategy", module = "atomr_agents._native.instruction")]
#[derive(Clone)]
pub struct PyInstructionStrategy {
    pub(crate) inner: Arc<dyn InstructionStrategy>,
}

#[pymethods]
impl PyInstructionStrategy {
    fn __repr__(&self) -> String {
        "InstructionStrategy(handle)".into()
    }
}

pub(crate) struct PyInstructionStrategyAdapter {
    target: Arc<PyObject>,
}

#[async_trait]
impl InstructionStrategy for PyInstructionStrategyAdapter {
    async fn render(
        &self,
        ctx: &AgentContext,
        budget: &mut TokenBudget,
    ) -> AgentResult<RenderedInstructions> {
        let target = self.target.clone();
        let ctx_owned = ctx.clone();
        let budget_remaining = budget.remaining;
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let ctx_dict = agent_context_to_pydict(py, &ctx_owned)?;
            let bud = PyDict::new_bound(py);
            bud.set_item("remaining", budget_remaining)?;
            let instance: Bound<'_, PyAny> = if bound.hasattr("render")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("render")?.call1((ctx_dict, bud))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("instruction render: {e}")))?;
        let final_val = await_if_coro(coro_or_val).await?;
        Python::with_gil(|py| -> PyResult<RenderedInstructions> {
            let bound = final_val.bind(py);
            // Accept either a PyRenderedInstructions or a dict.
            if let Ok(r) = bound.extract::<PyRenderedInstructions>() {
                Ok(r.inner)
            } else {
                let d: &Bound<'_, PyDict> = bound.downcast()?;
                let sp: String = d
                    .get_item("system_prompt")?
                    .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err("missing 'system_prompt'"))?
                    .extract()?;
                let est: u32 = match d.get_item("estimated_tokens")? {
                    Some(v) => v.extract()?,
                    None => 0,
                };
                Ok(RenderedInstructions {
                    system_prompt: sp,
                    estimated_tokens: est,
                })
            }
        })
        .map_err(|e| AgentError::Internal(format!("instruction render result: {e}")))
    }
}

#[pyfunction]
fn instruction_strategy_from_factory(key: String) -> PyResult<PyInstructionStrategy> {
    let target = crate::guest::must_lookup("strategy:instruction", &key)
        .or_else(|_| crate::guest::must_lookup("instruction", &key))?;
    Ok(PyInstructionStrategy {
        inner: Arc::new(PyInstructionStrategyAdapter { target }),
    })
}

// ----- Static task / behavior strategies ----------------------------------

/// Convenience: a static `TaskStrategy` that always resolves to the
/// given string. Returned as an opaque dyn handle used by
/// `composed_instruction`.
#[pyclass(name = "TaskStrategy", module = "atomr_agents._native.instruction")]
#[derive(Clone)]
pub struct PyTaskStrategy {
    pub(crate) inner: Arc<dyn TaskStrategy>,
}
#[pymethods]
impl PyTaskStrategy {
    fn __repr__(&self) -> String {
        "TaskStrategy(handle)".into()
    }
}

#[pyclass(name = "BehaviorStrategy", module = "atomr_agents._native.instruction")]
#[derive(Clone)]
pub struct PyBehaviorStrategy {
    pub(crate) inner: Arc<dyn BehaviorStrategy>,
}
#[pymethods]
impl PyBehaviorStrategy {
    fn __repr__(&self) -> String {
        "BehaviorStrategy(handle)".into()
    }
}

#[pyfunction]
fn task_static(text: String) -> PyTaskStrategy {
    PyTaskStrategy {
        inner: Arc::new(StaticTaskStrategy(text)),
    }
}

#[pyfunction]
fn behavior_static(text: String) -> PyBehaviorStrategy {
    PyBehaviorStrategy {
        inner: Arc::new(StaticBehaviorStrategy(text)),
    }
}

// ----- Task / Behavior adapters from Python ------------------------------

pub(crate) struct PyTaskStrategyAdapter {
    target: Arc<PyObject>,
}

#[async_trait]
impl TaskStrategy for PyTaskStrategyAdapter {
    async fn resolve(&self, ctx: &AgentContext, budget: &mut TokenBudget) -> AgentResult<String> {
        let target = self.target.clone();
        let ctx_owned = ctx.clone();
        let budget_remaining = budget.remaining;
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let ctx_dict = agent_context_to_pydict(py, &ctx_owned)?;
            let bud = PyDict::new_bound(py);
            bud.set_item("remaining", budget_remaining)?;
            let instance: Bound<'_, PyAny> = if bound.hasattr("resolve")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("resolve")?.call1((ctx_dict, bud))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("task resolve: {e}")))?;
        let final_val = await_if_coro(coro_or_val).await?;
        Python::with_gil(|py| final_val.bind(py).extract::<String>())
            .map_err(|e| AgentError::Internal(format!("task resolve result: {e}")))
    }
}

pub(crate) struct PyBehaviorStrategyAdapter {
    target: Arc<PyObject>,
}

#[async_trait]
impl BehaviorStrategy for PyBehaviorStrategyAdapter {
    async fn resolve(&self, ctx: &AgentContext, budget: &mut TokenBudget) -> AgentResult<String> {
        let target = self.target.clone();
        let ctx_owned = ctx.clone();
        let budget_remaining = budget.remaining;
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let ctx_dict = agent_context_to_pydict(py, &ctx_owned)?;
            let bud = PyDict::new_bound(py);
            bud.set_item("remaining", budget_remaining)?;
            let instance: Bound<'_, PyAny> = if bound.hasattr("resolve")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("resolve")?.call1((ctx_dict, bud))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("behavior resolve: {e}")))?;
        let final_val = await_if_coro(coro_or_val).await?;
        Python::with_gil(|py| final_val.bind(py).extract::<String>())
            .map_err(|e| AgentError::Internal(format!("behavior resolve result: {e}")))
    }
}

#[pyfunction]
fn task_from_factory(key: String) -> PyResult<PyTaskStrategy> {
    let target = crate::guest::must_lookup("strategy:task", &key)?;
    Ok(PyTaskStrategy {
        inner: Arc::new(PyTaskStrategyAdapter { target }),
    })
}

#[pyfunction]
fn behavior_from_factory(key: String) -> PyResult<PyBehaviorStrategy> {
    let target = crate::guest::must_lookup("strategy:behavior", &key)?;
    Ok(PyBehaviorStrategy {
        inner: Arc::new(PyBehaviorStrategyAdapter { target }),
    })
}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "instruction")?;
    m.add_class::<PyRenderedInstructions>()?;
    m.add_class::<PyRenderedMessage>()?;
    m.add_class::<PyStringTemplate>()?;
    m.add_class::<PyMessageTemplate>()?;
    m.add_class::<PyChatPromptTemplate>()?;
    m.add_class::<PyChatPromptTemplateBuilder>()?;
    m.add_class::<PyExample>()?;
    m.add_class::<PyExampleSelector>()?;
    m.add_class::<PyFewShotChatTemplate>()?;
    m.add_class::<PyInstructionStrategy>()?;
    m.add_class::<PyTaskStrategy>()?;
    m.add_class::<PyBehaviorStrategy>()?;
    m.add_function(wrap_pyfunction!(length_based_selector, &m)?)?;
    m.add_function(wrap_pyfunction!(semantic_similarity_selector, &m)?)?;
    m.add_function(wrap_pyfunction!(instruction_strategy_from_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(task_static, &m)?)?;
    m.add_function(wrap_pyfunction!(behavior_static, &m)?)?;
    m.add_function(wrap_pyfunction!(task_from_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(behavior_from_factory, &m)?)?;
    // Alias MessagesPlaceholder = MessageTemplate.placeholder factory.
    m.add("MessagesPlaceholder", m.getattr("MessageTemplate")?)?;
    parent.add_submodule(&m)?;
    Ok(())
}
