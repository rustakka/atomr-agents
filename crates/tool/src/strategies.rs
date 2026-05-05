use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::CallableHandle;
use atomr_agents_core::{AgentContext, Result, TokenBudget};
use atomr_agents_strategy::{ToolRef, ToolStrategy};

use crate::r#trait::{DynTool, ToolCallable};

fn dyn_tool_to_callable(t: DynTool) -> CallableHandle {
    // Wrap the trait object in a `ToolCallable` (newtype that
    // implements `Callable`). We need a helper that erases the
    // generic — a small adapter struct.
    Arc::new(DynToolCallable { inner: t })
}

struct DynToolCallable {
    inner: DynTool,
}

#[async_trait::async_trait]
impl atomr_agents_callable::Callable for DynToolCallable {
    async fn call(
        &self,
        input: atomr_agents_core::Value,
        ctx: atomr_agents_core::CallCtx,
    ) -> atomr_agents_core::Result<atomr_agents_core::Value> {
        let invoke_ctx = atomr_agents_core::InvokeCtx {
            call: ctx,
            tool_call_id: String::new(),
            raw_args: input.clone(),
        };
        self.inner.invoke(input, &invoke_ctx).await
    }

    fn label(&self) -> &str {
        &self.inner.descriptor().name
    }
}

/// v0: hand-picked fixed list of tools.
pub struct StaticToolStrategy {
    tools: Vec<DynTool>,
}

impl StaticToolStrategy {
    pub fn new(tools: Vec<DynTool>) -> Self {
        Self { tools }
    }
}

#[async_trait]
impl ToolStrategy for StaticToolStrategy {
    async fn select(&self, _ctx: &AgentContext, _budget: &mut TokenBudget) -> Result<Vec<ToolRef>> {
        Ok(self
            .tools
            .iter()
            .map(|t| {
                let d = t.descriptor();
                ToolRef {
                    id: d.id.clone(),
                    name: d.name.clone(),
                    handle: dyn_tool_to_callable(t.clone()),
                }
            })
            .collect())
    }
}

/// v1: lexical filter. Returns tools whose name or description
/// contains any of the keywords found in the user's turn input.
/// (Substring match in v0; promote to TF-IDF later.)
pub struct KeywordToolStrategy {
    tools: Vec<DynTool>,
    /// Maximum number of tools to return.
    max_tools: usize,
}

impl KeywordToolStrategy {
    pub fn new(tools: Vec<DynTool>, max_tools: usize) -> Self {
        Self { tools, max_tools }
    }
}

#[async_trait]
impl ToolStrategy for KeywordToolStrategy {
    async fn select(&self, ctx: &AgentContext, _budget: &mut TokenBudget) -> Result<Vec<ToolRef>> {
        let needle = ctx.turn.user.to_lowercase();
        let words: Vec<&str> = needle
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect();
        let mut scored: Vec<(usize, &DynTool)> = self
            .tools
            .iter()
            .map(|t| {
                let d = t.descriptor();
                let hay = format!("{} {}", d.name.to_lowercase(), d.description.to_lowercase());
                let score = words.iter().filter(|w| hay.contains(*w)).count();
                (score, t)
            })
            .filter(|(s, _)| *s > 0)
            .collect();
        scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        scored.truncate(self.max_tools);
        Ok(scored
            .into_iter()
            .map(|(_, t)| {
                let d = t.descriptor();
                ToolRef {
                    id: d.id.clone(),
                    name: d.name.clone(),
                    handle: dyn_tool_to_callable(t.clone()),
                }
            })
            .collect())
    }
}

// silence unused-import warning for ToolCallable; it's part of the
// public surface but only constructed by direct callers.
#[allow(dead_code)]
fn _keep_tool_callable_alive<T: crate::r#trait::Tool>(t: T) -> ToolCallable<T> {
    ToolCallable::new(t)
}
