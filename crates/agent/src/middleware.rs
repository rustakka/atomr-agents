//! Agent middleware — `create_agent`-style hooks around the per-turn
//! pipeline.
//!
//! Each middleware exposes optional hooks for: agent-start,
//! model-call (before/after), tool-call (before/after), agent-end,
//! and dynamic-prompt. The agent runs them in registration order for
//! `before_*` hooks and reverse order for `after_*` hooks (Tower
//! convention). Stock implementations cover logging, retry,
//! rate-limit, redaction, and tool-error recovery.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use atomr_agents_core::{AgentError, AgentId, Result, Value};
use atomr_infer_core::batch::ExecuteBatch;
use parking_lot::Mutex;

use crate::inference::TurnResult;

#[async_trait]
pub trait AgentMiddleware: Send + Sync + 'static {
    async fn before_agent(&self, _agent_id: &AgentId, _user: &str) -> Result<()> {
        Ok(())
    }
    async fn before_model_call(&self, _batch: &mut ExecuteBatch) -> Result<()> {
        Ok(())
    }
    async fn after_model_call(&self, _result: &mut TurnResult) -> Result<()> {
        Ok(())
    }
    async fn before_tool_call(&self, _name: &str, _args: &mut Value) -> Result<()> {
        Ok(())
    }
    async fn after_tool_call(&self, _name: &str, _result: &mut Result<Value>) -> Result<()> {
        Ok(())
    }
    async fn after_agent(&self, _result: &mut TurnResult) -> Result<()> {
        Ok(())
    }
    /// If `Some`, replaces the rendered system prompt for this turn.
    async fn dynamic_prompt(&self, _agent_id: &AgentId, _user: &str) -> Result<Option<String>> {
        Ok(None)
    }
}

/// Convenience container — registered middlewares + helpers to run them.
#[derive(Default, Clone)]
pub struct MiddlewareStack {
    inner: Vec<Arc<dyn AgentMiddleware>>,
}

impl MiddlewareStack {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(mut self, m: Arc<dyn AgentMiddleware>) -> Self {
        self.inner.push(m);
        self
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn AgentMiddleware>> {
        self.inner.iter()
    }

    pub fn iter_rev(&self) -> impl Iterator<Item = &Arc<dyn AgentMiddleware>> {
        self.inner.iter().rev()
    }

    pub async fn run_before_agent(&self, agent_id: &AgentId, user: &str) -> Result<()> {
        for m in self.iter() {
            m.before_agent(agent_id, user).await?;
        }
        Ok(())
    }

    pub async fn run_before_model_call(&self, batch: &mut ExecuteBatch) -> Result<()> {
        for m in self.iter() {
            m.before_model_call(batch).await?;
        }
        Ok(())
    }

    pub async fn run_after_model_call(&self, result: &mut TurnResult) -> Result<()> {
        for m in self.iter_rev() {
            m.after_model_call(result).await?;
        }
        Ok(())
    }

    pub async fn run_before_tool_call(&self, name: &str, args: &mut Value) -> Result<()> {
        for m in self.iter() {
            m.before_tool_call(name, args).await?;
        }
        Ok(())
    }

    pub async fn run_after_tool_call(&self, name: &str, result: &mut Result<Value>) -> Result<()> {
        for m in self.iter_rev() {
            m.after_tool_call(name, result).await?;
        }
        Ok(())
    }

    pub async fn run_after_agent(&self, result: &mut TurnResult) -> Result<()> {
        for m in self.iter_rev() {
            m.after_agent(result).await?;
        }
        Ok(())
    }

    pub async fn run_dynamic_prompt(&self, agent_id: &AgentId, user: &str) -> Result<Option<String>> {
        // Last `Some` wins (later middlewares override earlier).
        let mut out: Option<String> = None;
        for m in self.iter() {
            if let Some(s) = m.dynamic_prompt(agent_id, user).await? {
                out = Some(s);
            }
        }
        Ok(out)
    }
}

// --------------------------------------------------------------------
// LoggingMiddleware
// --------------------------------------------------------------------

#[derive(Default, Clone)]
pub struct LoggingMiddleware {
    pub log: Arc<Mutex<Vec<String>>>,
}

impl LoggingMiddleware {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn lines(&self) -> Vec<String> {
        self.log.lock().clone()
    }
}

#[async_trait]
impl AgentMiddleware for LoggingMiddleware {
    async fn before_agent(&self, agent_id: &AgentId, user: &str) -> Result<()> {
        self.log
            .lock()
            .push(format!("before_agent {} '{}'", agent_id.as_str(), user));
        Ok(())
    }
    async fn before_model_call(&self, batch: &mut ExecuteBatch) -> Result<()> {
        self.log
            .lock()
            .push(format!("before_model_call model={}", batch.model));
        Ok(())
    }
    async fn after_model_call(&self, result: &mut TurnResult) -> Result<()> {
        self.log.lock().push(format!(
            "after_model_call out_tokens={}",
            result.usage.output_tokens
        ));
        Ok(())
    }
    async fn before_tool_call(&self, name: &str, _args: &mut Value) -> Result<()> {
        self.log.lock().push(format!("before_tool_call {name}"));
        Ok(())
    }
    async fn after_tool_call(&self, name: &str, result: &mut Result<Value>) -> Result<()> {
        let ok = result.is_ok();
        self.log.lock().push(format!("after_tool_call {name} ok={ok}"));
        Ok(())
    }
    async fn after_agent(&self, _r: &mut TurnResult) -> Result<()> {
        self.log.lock().push("after_agent".into());
        Ok(())
    }
}

// --------------------------------------------------------------------
// RateLimitMiddleware — token-bucket
// --------------------------------------------------------------------

pub struct RateLimitMiddleware {
    capacity: u32,
    refill_per_sec: u32,
    state: Mutex<BucketState>,
}

struct BucketState {
    tokens: f32,
    last: Instant,
}

impl RateLimitMiddleware {
    pub fn new(capacity: u32, refill_per_sec: u32) -> Self {
        Self {
            capacity,
            refill_per_sec,
            state: Mutex::new(BucketState {
                tokens: capacity as f32,
                last: Instant::now(),
            }),
        }
    }

    fn try_take(&self) -> bool {
        let mut s = self.state.lock();
        let now = Instant::now();
        let elapsed = now.duration_since(s.last).as_secs_f32();
        s.tokens = (s.tokens + elapsed * self.refill_per_sec as f32).min(self.capacity as f32);
        s.last = now;
        if s.tokens >= 1.0 {
            s.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[async_trait]
impl AgentMiddleware for RateLimitMiddleware {
    async fn before_model_call(&self, _batch: &mut ExecuteBatch) -> Result<()> {
        let mut waited = Duration::ZERO;
        while !self.try_take() {
            let backoff = Duration::from_millis(50);
            tokio::time::sleep(backoff).await;
            waited += backoff;
            if waited > Duration::from_secs(10) {
                return Err(AgentError::Inference("rate-limit: gave up after 10s".into()));
            }
        }
        Ok(())
    }
}

// --------------------------------------------------------------------
// RedactionMiddleware — replace patterns in the *user* message of the
// outgoing batch with a placeholder.
// --------------------------------------------------------------------

pub struct RedactionMiddleware {
    pub patterns: Vec<String>,
    pub replacement: String,
}

impl RedactionMiddleware {
    pub fn new(patterns: Vec<String>, replacement: impl Into<String>) -> Self {
        Self {
            patterns,
            replacement: replacement.into(),
        }
    }
}

#[async_trait]
impl AgentMiddleware for RedactionMiddleware {
    async fn before_model_call(&self, batch: &mut ExecuteBatch) -> Result<()> {
        for msg in &mut batch.messages {
            if let atomr_infer_core::batch::MessageContent::Text(t) = &mut msg.content {
                for p in &self.patterns {
                    *t = t.replace(p, &self.replacement);
                }
            }
        }
        Ok(())
    }
}

// --------------------------------------------------------------------
// ToolErrorRecoveryMiddleware — convert tool errors into
// model-readable "tool error" content so the model can recover.
// --------------------------------------------------------------------

pub struct ToolErrorRecoveryMiddleware;

#[async_trait]
impl AgentMiddleware for ToolErrorRecoveryMiddleware {
    async fn after_tool_call(&self, name: &str, result: &mut Result<Value>) -> Result<()> {
        if let Err(e) = result {
            let payload = serde_json::json!({ "tool_error": true, "tool": name, "message": e.to_string() });
            *result = Ok(payload);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_infer_core::batch::{ExecuteBatch, MessageContent, SamplingParams};

    fn batch(model: &str, user_text: &str) -> ExecuteBatch {
        ExecuteBatch {
            request_id: "r".into(),
            model: model.into(),
            messages: vec![atomr_infer_core::batch::Message {
                role: atomr_infer_core::batch::Role::User,
                content: MessageContent::Text(user_text.into()),
            }],
            sampling: SamplingParams::default(),
            stream: false,
            estimated_tokens: 1,
        }
    }

    #[tokio::test]
    async fn logging_records_each_phase() {
        let m: Arc<dyn AgentMiddleware> = Arc::new(LoggingMiddleware::new());
        let stack = MiddlewareStack::new().push(m.clone());
        stack.run_before_agent(&AgentId::from("a"), "hi").await.unwrap();
        let mut b = batch("mock", "hi");
        stack.run_before_model_call(&mut b).await.unwrap();
        let m_dc: &LoggingMiddleware = unsafe { &*(Arc::as_ptr(&m) as *const LoggingMiddleware) };
        assert!(m_dc.lines().iter().any(|l| l.starts_with("before_agent")));
        assert!(m_dc.lines().iter().any(|l| l.starts_with("before_model_call")));
    }

    #[tokio::test]
    async fn redaction_strips_patterns() {
        let stack = MiddlewareStack::new().push(Arc::new(RedactionMiddleware::new(
            vec!["secret".into()],
            "[redacted]",
        )));
        let mut b = batch("mock", "the secret is out");
        stack.run_before_model_call(&mut b).await.unwrap();
        let MessageContent::Text(t) = &b.messages[0].content else {
            panic!("expected text");
        };
        assert_eq!(t, "the [redacted] is out");
    }

    #[tokio::test]
    async fn tool_error_recovery_converts_err_to_payload() {
        let stack = MiddlewareStack::new().push(Arc::new(ToolErrorRecoveryMiddleware));
        let mut r: Result<Value> = Err(AgentError::Tool("boom".into()));
        stack.run_after_tool_call("calc", &mut r).await.unwrap();
        let v = r.unwrap();
        assert_eq!(v["tool_error"], true);
        assert_eq!(v["tool"], "calc");
    }

    #[tokio::test]
    async fn rate_limit_allows_burst_then_blocks() {
        let m: Arc<dyn AgentMiddleware> = Arc::new(RateLimitMiddleware::new(2, 1));
        let stack = MiddlewareStack::new().push(m);
        let mut b = batch("m", "hi");
        // Two within capacity → quick.
        let t0 = Instant::now();
        stack.run_before_model_call(&mut b).await.unwrap();
        stack.run_before_model_call(&mut b).await.unwrap();
        let warm = t0.elapsed();
        assert!(warm < Duration::from_millis(100));
        // Third should wait at least one refill tick.
        let t1 = Instant::now();
        stack.run_before_model_call(&mut b).await.unwrap();
        let cold = t1.elapsed();
        assert!(cold >= Duration::from_millis(40));
    }
}
