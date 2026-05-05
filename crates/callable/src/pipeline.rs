//! `Pipeline` — composable builder over `Callable`s.
//!
//! LCEL's `prompt | model | parser` becomes
//! `Pipeline::from(prompt).then(model).then(parser).build()`.
//! Fan-out becomes `.fan_out({"a": ca, "b": cb})`. Every result is
//! itself a `Callable`, so pipelines compose recursively.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentError, CallCtx, Result, Value};

use crate::{Callable, CallableHandle};

#[derive(Clone, Copy, Debug)]
enum StageKind {
    Sequential,
    /// Adds a key to the input dict; the original input is preserved.
    Assign,
}

#[derive(Clone)]
struct Stage {
    kind: StageKind,
    /// Used by `Assign`/`Passthrough`.
    key: Option<String>,
    callable: CallableHandle,
}

/// Builder over a sequence of `Callable`s.
pub struct Pipeline {
    stages: Vec<Stage>,
    label: String,
}

impl Pipeline {
    pub fn from(c: CallableHandle) -> Self {
        let label = c.label().to_string();
        Self {
            stages: vec![Stage { kind: StageKind::Sequential, key: None, callable: c }],
            label,
        }
    }

    /// `prompt | model` — chain another stage.
    pub fn then(mut self, c: CallableHandle) -> Self {
        self.label = format!("{} | {}", self.label, c.label());
        self.stages.push(Stage { kind: StageKind::Sequential, key: None, callable: c });
        self
    }

    /// Pass input through unchanged. Useful as a starting node.
    pub fn passthrough(self) -> Self {
        let identity: CallableHandle = Arc::new(crate::FnCallable::labeled(
            "passthrough",
            |v: Value, _ctx| async move { Ok(v) },
        ));
        if self.stages.is_empty() {
            return Pipeline::from(identity);
        }
        self.then(identity)
    }

    /// `RunnablePassthrough.assign(key=fn)` — run `c` on the *current*
    /// input and add the result under `key`, leaving original input
    /// fields intact.
    pub fn assign(mut self, key: impl Into<String>, c: CallableHandle) -> Self {
        let key = key.into();
        self.label = format!("{}.assign({})", self.label, key);
        self.stages.push(Stage { kind: StageKind::Assign, key: Some(key), callable: c });
        self
    }

    /// `RunnableParallel({a: ca, b: cb})` as an inline stage. Each
    /// branch runs concurrently on the current input; output is a
    /// JSON object keyed by branch name.
    pub fn fan_out_with(mut self, branches: Vec<(String, CallableHandle)>) -> Self {
        let names: Vec<&str> = branches.iter().map(|(k, _)| k.as_str()).collect();
        let label = format!("{} | fan_out({})", self.label, names.join(","));
        let stage_callable = FanOutCallable::new(branches);
        let handle: CallableHandle = Arc::new(stage_callable);
        self.label = label;
        self.stages
            .push(Stage { kind: StageKind::Sequential, key: None, callable: handle });
        self
    }

    pub fn build(self) -> CallableHandle {
        Arc::new(BuiltPipeline { stages: self.stages, label: self.label })
    }
}

struct BuiltPipeline {
    stages: Vec<Stage>,
    label: String,
}

#[async_trait]
impl Callable for BuiltPipeline {
    async fn call(&self, input: Value, ctx: CallCtx) -> Result<Value> {
        let mut current = input;
        for stage in &self.stages {
            match stage.kind {
                StageKind::Sequential => {
                    current = stage.callable.call(current, ctx.clone()).await?;
                }
                StageKind::Assign => {
                    let key = stage
                        .key
                        .as_ref()
                        .ok_or_else(|| AgentError::Internal("assign without key".into()))?;
                    let derived = stage.callable.call(current.clone(), ctx.clone()).await?;
                    let mut obj = match current {
                        Value::Object(m) => m,
                        other => {
                            let mut m = serde_json::Map::new();
                            m.insert("input".into(), other);
                            m
                        }
                    };
                    obj.insert(key.clone(), derived);
                    current = Value::Object(obj);
                }
            }
        }
        Ok(current)
    }

    fn label(&self) -> &str {
        &self.label
    }
}

/// Standalone fan-out factory. `fan_out([("a", ca), ("b", cb)])` —
/// equivalent to LCEL's `RunnableParallel` outside of a pipeline.
pub fn fan_out(branches: Vec<(String, CallableHandle)>) -> CallableHandle {
    Arc::new(FanOutCallable::new(branches))
}

struct FanOutCallable {
    branches: BTreeMap<String, CallableHandle>,
    label: String,
}

impl FanOutCallable {
    fn new(branches: Vec<(String, CallableHandle)>) -> Self {
        let label = format!(
            "fan_out({})",
            branches.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(",")
        );
        Self { branches: branches.into_iter().collect(), label }
    }
}

#[async_trait]
impl Callable for FanOutCallable {
    async fn call(&self, input: Value, ctx: CallCtx) -> Result<Value> {
        let mut handles = Vec::with_capacity(self.branches.len());
        for (k, c) in &self.branches {
            let k = k.clone();
            let c = c.clone();
            let inp = input.clone();
            let ctx = ctx.clone();
            handles.push(tokio::spawn(async move {
                let out = c.call(inp, ctx).await?;
                Ok::<_, AgentError>((k, out))
            }));
        }
        let mut out = serde_json::Map::new();
        for h in handles {
            let (k, v) = h.await.map_err(|e| AgentError::Internal(e.to_string()))??;
            out.insert(k, v);
        }
        Ok(Value::Object(out))
    }

    fn label(&self) -> &str {
        &self.label
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FnCallable;
    use atomr_agents_core::{IterationBudget, MoneyBudget, TimeBudget, TokenBudget};
    use std::time::Duration;

    fn ctx() -> CallCtx {
        CallCtx {
            agent_id: None,
            tokens: TokenBudget::new(1000),
            time: TimeBudget::new(Duration::from_secs(10)),
            money: MoneyBudget::from_usd(1.0),
            iterations: IterationBudget::new(10),
            trace: vec![],
        }
    }

    fn echo(label: &'static str) -> CallableHandle {
        Arc::new(FnCallable::labeled(label, |v: Value, _ctx| async move { Ok(v) }))
    }

    fn append_str(label: &'static str, suffix: &'static str) -> CallableHandle {
        Arc::new(FnCallable::labeled(label, move |v: Value, _ctx| async move {
            let s = v.as_str().unwrap_or("").to_string() + suffix;
            Ok(Value::String(s))
        }))
    }

    #[tokio::test]
    async fn pipeline_then_chains_sequentially() {
        let p = Pipeline::from(append_str("a", "A"))
            .then(append_str("b", "B"))
            .then(append_str("c", "C"))
            .build();
        let out = p.call(Value::String(String::new()), ctx()).await.unwrap();
        assert_eq!(out, Value::String("ABC".into()));
    }

    #[tokio::test]
    async fn fan_out_runs_branches_in_parallel() {
        let p = Pipeline::from(echo("seed"))
            .fan_out_with(vec![
                ("upper".into(), append_str("u", "U")),
                ("lower".into(), append_str("l", "l")),
            ])
            .build();
        let out = p.call(Value::String("x".into()), ctx()).await.unwrap();
        assert_eq!(out["upper"], Value::String("xU".into()));
        assert_eq!(out["lower"], Value::String("xl".into()));
    }

    #[tokio::test]
    async fn assign_adds_key_keeping_input_fields() {
        let derive = Arc::new(FnCallable::labeled("len", |v: Value, _ctx| async move {
            let n = v.as_object().map(|m| m.len()).unwrap_or(0);
            Ok(Value::from(n))
        }));
        let p = Pipeline::from(echo("seed")).assign("size", derive).build();
        let out = p
            .call(serde_json::json!({"a": 1, "b": 2}), ctx())
            .await
            .unwrap();
        assert_eq!(out["a"], Value::from(1));
        assert_eq!(out["size"], Value::from(2));
    }
}
