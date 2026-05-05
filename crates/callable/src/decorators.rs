//! Decorators over `Callable`.
//!
//! Each is itself a `Callable`, so they can be inserted anywhere a
//! `CallableHandle` is expected — including as middleware around
//! single steps in a `Pipeline`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use atomr_agents_core::{AgentError, CallCtx, Result, Value};

use crate::{Callable, CallableHandle, FnCallable};

// --------------------------------------------------------------------
// WithRetry
// --------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub initial_backoff: Duration,
    pub backoff_multiplier: f32,
    pub max_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(50),
            backoff_multiplier: 2.0,
            max_backoff: Duration::from_secs(5),
        }
    }
}

pub struct WithRetry {
    inner: CallableHandle,
    policy: RetryPolicy,
    label: String,
}

impl WithRetry {
    pub fn new(inner: CallableHandle, policy: RetryPolicy) -> Self {
        let label = format!("retry({})", inner.label());
        Self { inner, policy, label }
    }
}

pub fn with_retry(inner: CallableHandle, policy: RetryPolicy) -> CallableHandle {
    Arc::new(WithRetry::new(inner, policy))
}

#[async_trait]
impl Callable for WithRetry {
    async fn call(&self, input: Value, ctx: CallCtx) -> Result<Value> {
        let mut delay = self.policy.initial_backoff;
        let mut last_err: Option<AgentError> = None;
        for attempt in 0..self.policy.max_attempts {
            match self.inner.call(input.clone(), ctx.clone()).await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    last_err = Some(e);
                    if attempt + 1 == self.policy.max_attempts {
                        break;
                    }
                    tokio::time::sleep(delay).await;
                    let next_ms = (delay.as_millis() as f32 * self.policy.backoff_multiplier) as u64;
                    delay = Duration::from_millis(next_ms).min(self.policy.max_backoff);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| AgentError::Internal("retry exhausted with no error".into())))
    }

    fn label(&self) -> &str {
        &self.label
    }
}

// --------------------------------------------------------------------
// WithFallbacks
// --------------------------------------------------------------------

pub struct WithFallbacks {
    primary: CallableHandle,
    alternates: Vec<CallableHandle>,
    label: String,
}

impl WithFallbacks {
    pub fn new(primary: CallableHandle, alternates: Vec<CallableHandle>) -> Self {
        let label = format!("fallback({})", primary.label());
        Self {
            primary,
            alternates,
            label,
        }
    }
}

pub fn with_fallbacks(primary: CallableHandle, alternates: Vec<CallableHandle>) -> CallableHandle {
    Arc::new(WithFallbacks::new(primary, alternates))
}

#[async_trait]
impl Callable for WithFallbacks {
    async fn call(&self, input: Value, ctx: CallCtx) -> Result<Value> {
        if let Ok(v) = self.primary.call(input.clone(), ctx.clone()).await {
            return Ok(v);
        }
        let mut last_err = None;
        for alt in &self.alternates {
            match alt.call(input.clone(), ctx.clone()).await {
                Ok(v) => return Ok(v),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| AgentError::Internal("fallbacks exhausted".into())))
    }

    fn label(&self) -> &str {
        &self.label
    }
}

// --------------------------------------------------------------------
// WithConfig
// --------------------------------------------------------------------

#[derive(Clone, Default, Debug)]
pub struct RunConfig {
    pub run_name: Option<String>,
    pub tags: Vec<String>,
    /// Free-form metadata, JSON-encoded.
    pub metadata: serde_json::Map<String, Value>,
}

pub struct WithConfig {
    inner: CallableHandle,
    config: RunConfig,
    label: String,
}

impl WithConfig {
    pub fn new(inner: CallableHandle, config: RunConfig) -> Self {
        let label = config
            .run_name
            .clone()
            .unwrap_or_else(|| format!("config({})", inner.label()));
        Self { inner, config, label }
    }
}

pub fn with_config(inner: CallableHandle, config: RunConfig) -> CallableHandle {
    Arc::new(WithConfig::new(inner, config))
}

#[async_trait]
impl Callable for WithConfig {
    async fn call(&self, input: Value, ctx: CallCtx) -> Result<Value> {
        let mut ctx = ctx;
        if let Some(name) = &self.config.run_name {
            ctx.trace.push(format!("run:{name}"));
        }
        for t in &self.config.tags {
            ctx.trace.push(format!("tag:{t}"));
        }
        self.inner.call(input, ctx).await
    }

    fn label(&self) -> &str {
        &self.label
    }
}

// --------------------------------------------------------------------
// WithTimeout
// --------------------------------------------------------------------

pub struct WithTimeout {
    inner: CallableHandle,
    duration: Duration,
    label: String,
}

impl WithTimeout {
    pub fn new(inner: CallableHandle, duration: Duration) -> Self {
        let label = format!("timeout({})", inner.label());
        Self {
            inner,
            duration,
            label,
        }
    }
}

pub fn with_timeout(inner: CallableHandle, duration: Duration) -> CallableHandle {
    Arc::new(WithTimeout::new(inner, duration))
}

#[async_trait]
impl Callable for WithTimeout {
    async fn call(&self, input: Value, ctx: CallCtx) -> Result<Value> {
        match tokio::time::timeout(self.duration, self.inner.call(input, ctx)).await {
            Ok(r) => r,
            Err(_) => Err(AgentError::Internal(format!(
                "timed out after {:?}",
                self.duration
            ))),
        }
    }

    fn label(&self) -> &str {
        &self.label
    }
}

// --------------------------------------------------------------------
// Branch — RunnableBranch analogue
// --------------------------------------------------------------------

pub struct Branch {
    predicate: Arc<dyn Fn(&Value) -> bool + Send + Sync + 'static>,
    if_true: CallableHandle,
    if_false: CallableHandle,
    label: String,
}

impl Branch {
    pub fn new<F>(predicate: F, if_true: CallableHandle, if_false: CallableHandle) -> Self
    where
        F: Fn(&Value) -> bool + Send + Sync + 'static,
    {
        let label = format!("branch({} | {})", if_true.label(), if_false.label());
        Self {
            predicate: Arc::new(predicate),
            if_true,
            if_false,
            label,
        }
    }
}

#[async_trait]
impl Callable for Branch {
    async fn call(&self, input: Value, ctx: CallCtx) -> Result<Value> {
        if (self.predicate)(&input) {
            self.if_true.call(input, ctx).await
        } else {
            self.if_false.call(input, ctx).await
        }
    }

    fn label(&self) -> &str {
        &self.label
    }
}

// --------------------------------------------------------------------
// Lambda — RunnableLambda alias
// --------------------------------------------------------------------

/// Type-alias for users who prefer the LangChain name.
pub type Lambda<F> = FnCallable<F>;

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_core::{IterationBudget, MoneyBudget, TimeBudget, TokenBudget};
    use std::sync::atomic::{AtomicU32, Ordering};

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

    #[tokio::test]
    async fn retry_succeeds_after_two_failures() {
        let count = Arc::new(AtomicU32::new(0));
        let count_clone = count.clone();
        let flaky = Arc::new(FnCallable::labeled("flaky", move |v: Value, _ctx| {
            let count = count_clone.clone();
            async move {
                let n = count.fetch_add(1, Ordering::SeqCst);
                if n < 2 {
                    Err(AgentError::Internal(format!("attempt {n} failed")))
                } else {
                    Ok(v)
                }
            }
        }));
        let retried = with_retry(
            flaky,
            RetryPolicy {
                max_attempts: 5,
                initial_backoff: Duration::from_millis(1),
                backoff_multiplier: 1.0,
                max_backoff: Duration::from_millis(1),
            },
        );
        let out = retried.call(Value::from("ok"), ctx()).await.unwrap();
        assert_eq!(out, Value::from("ok"));
        assert_eq!(count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn retry_exhausts_then_errors() {
        let always_fail = Arc::new(FnCallable::labeled("nope", |_v: Value, _ctx| async {
            Err::<Value, _>(AgentError::Internal("boom".into()))
        }));
        let retried = with_retry(
            always_fail,
            RetryPolicy {
                max_attempts: 2,
                initial_backoff: Duration::from_millis(1),
                backoff_multiplier: 1.0,
                max_backoff: Duration::from_millis(1),
            },
        );
        let r = retried.call(Value::Null, ctx()).await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn fallback_uses_alternate_after_primary_failure() {
        let primary = Arc::new(FnCallable::labeled("p", |_v: Value, _ctx| async {
            Err::<Value, _>(AgentError::Inference("primary down".into()))
        }));
        let alt = Arc::new(FnCallable::labeled("alt", |v: Value, _ctx| async move { Ok(v) }));
        let composed = with_fallbacks(primary, vec![alt]);
        let out = composed.call(Value::from(42), ctx()).await.unwrap();
        assert_eq!(out, Value::from(42));
    }

    #[tokio::test]
    async fn timeout_fires() {
        let slow = Arc::new(FnCallable::labeled("slow", |_v: Value, _ctx| async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok(Value::Null)
        }));
        let bounded = with_timeout(slow, Duration::from_millis(5));
        let r = bounded.call(Value::Null, ctx()).await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn config_pushes_run_name_and_tags_into_trace() {
        let inner = Arc::new(FnCallable::labeled(
            "inner",
            |_v: Value, ctx: CallCtx| async move { Ok(Value::from(serde_json::json!({"trace": ctx.trace}))) },
        ));
        let configured = with_config(
            inner,
            RunConfig {
                run_name: Some("my-run".into()),
                tags: vec!["alpha".into(), "beta".into()],
                metadata: Default::default(),
            },
        );
        let out = configured.call(Value::Null, ctx()).await.unwrap();
        let trace = out["trace"].as_array().unwrap();
        let s: Vec<String> = trace.iter().map(|v| v.as_str().unwrap().to_string()).collect();
        assert!(s.contains(&"run:my-run".to_string()));
        assert!(s.contains(&"tag:alpha".to_string()));
        assert!(s.contains(&"tag:beta".to_string()));
    }

    #[tokio::test]
    async fn branch_routes_on_predicate() {
        let big = Arc::new(FnCallable::labeled("big", |_v: Value, _ctx| async {
            Ok(Value::from("big"))
        }));
        let small = Arc::new(FnCallable::labeled("small", |_v: Value, _ctx| async {
            Ok(Value::from("small"))
        }));
        let b = Branch::new(|v: &Value| v.as_i64().unwrap_or(0) > 10, big, small);
        assert_eq!(b.call(Value::from(42), ctx()).await.unwrap(), Value::from("big"));
        assert_eq!(b.call(Value::from(1), ctx()).await.unwrap(), Value::from("small"));
    }
}
