//! Auto-repair wrappers.
//!
//! `OutputFixingParser` calls a "repair" model with the malformed
//! output + the parser's format instructions. `RetryWithErrorParser`
//! re-prompts with the original prompt + the failure message.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentError, Result};

use crate::Parser;

#[async_trait]
pub trait RepairModel: Send + Sync + 'static {
    /// Given the original output and a hint (instructions or error),
    /// produce a corrected raw string.
    async fn repair(&self, original: &str, hint: &str) -> Result<String>;
}

pub struct OutputFixingParser<P, T>
where
    P: Parser<T> + 'static,
    T: Send + 'static,
{
    pub inner: Arc<P>,
    pub model: Arc<dyn RepairModel>,
    pub max_attempts: u32,
    _marker: std::marker::PhantomData<fn() -> T>,
}

impl<P, T> OutputFixingParser<P, T>
where
    P: Parser<T> + 'static,
    T: Send + 'static,
{
    pub fn new(inner: P, model: Arc<dyn RepairModel>, max_attempts: u32) -> Self {
        Self {
            inner: Arc::new(inner),
            model,
            max_attempts,
            _marker: std::marker::PhantomData,
        }
    }
}

#[async_trait]
impl<P, T> Parser<T> for OutputFixingParser<P, T>
where
    P: Parser<T> + 'static,
    T: Send + 'static,
{
    async fn parse(&self, raw: &str) -> Result<T> {
        let mut last_err = None;
        let mut current = raw.to_string();
        let instructions = self.inner.format_instructions();
        for _ in 0..self.max_attempts.max(1) {
            match self.inner.parse(&current).await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    last_err = Some(e);
                    let hint = format!(
                        "Output below failed format instructions. Re-emit corrected output.\n\nFormat:\n{instructions}\n\nFailed output:\n{current}"
                    );
                    current = self.model.repair(&current, &hint).await?;
                }
            }
        }
        Err(last_err.unwrap_or_else(|| AgentError::Internal("repair exhausted".into())))
    }
    fn format_instructions(&self) -> String {
        self.inner.format_instructions()
    }
}

pub struct RetryWithErrorParser<P, T>
where
    P: Parser<T> + 'static,
    T: Send + 'static,
{
    pub inner: Arc<P>,
    pub model: Arc<dyn RepairModel>,
    pub max_attempts: u32,
    /// The original prompt; passed to the repair model on each retry.
    pub original_prompt: String,
    _marker: std::marker::PhantomData<fn() -> T>,
}

impl<P, T> RetryWithErrorParser<P, T>
where
    P: Parser<T> + 'static,
    T: Send + 'static,
{
    pub fn new(
        inner: P,
        model: Arc<dyn RepairModel>,
        max_attempts: u32,
        original_prompt: impl Into<String>,
    ) -> Self {
        Self {
            inner: Arc::new(inner),
            model,
            max_attempts,
            original_prompt: original_prompt.into(),
            _marker: std::marker::PhantomData,
        }
    }
}

#[async_trait]
impl<P, T> Parser<T> for RetryWithErrorParser<P, T>
where
    P: Parser<T> + 'static,
    T: Send + 'static,
{
    async fn parse(&self, raw: &str) -> Result<T> {
        let mut current = raw.to_string();
        let mut last_err = None;
        for _ in 0..self.max_attempts.max(1) {
            match self.inner.parse(&current).await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    let hint = format!(
                        "Original prompt:\n{}\n\nError on previous output:\n{e}\n\nReply again, conforming to the prompt.",
                        self.original_prompt
                    );
                    last_err = Some(e);
                    current = self.model.repair(&current, &hint).await?;
                }
            }
        }
        Err(last_err.unwrap_or_else(|| AgentError::Internal("retry exhausted".into())))
    }
    fn format_instructions(&self) -> String {
        self.inner.format_instructions()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::basic::JsonParser;
    use atomr_agents_core::Value;
    use parking_lot::Mutex;

    struct ScriptedRepair {
        replies: Mutex<Vec<String>>,
    }
    #[async_trait]
    impl RepairModel for ScriptedRepair {
        async fn repair(&self, _original: &str, _hint: &str) -> Result<String> {
            let mut g = self.replies.lock();
            if g.is_empty() {
                return Err(AgentError::Inference("no scripted reply".into()));
            }
            Ok(g.remove(0))
        }
    }

    #[tokio::test]
    async fn output_fixing_recovers_after_one_repair() {
        let model = Arc::new(ScriptedRepair {
            replies: Mutex::new(vec![r#"{"ok": true}"#.to_string()]),
        });
        let p: OutputFixingParser<JsonParser, Value> = OutputFixingParser::new(JsonParser, model, 3);
        let v = p.parse("not json at all").await.unwrap();
        assert_eq!(v, serde_json::json!({"ok": true}));
    }

    #[tokio::test]
    async fn retry_with_error_re_prompts_with_failure() {
        let model = Arc::new(ScriptedRepair {
            replies: Mutex::new(vec!["still bad".into(), r#"{"ok": true}"#.to_string()]),
        });
        let p: RetryWithErrorParser<JsonParser, Value> =
            RetryWithErrorParser::new(JsonParser, model, 5, "Reply with JSON.");
        let v = p.parse("nope").await.unwrap();
        assert_eq!(v, serde_json::json!({"ok": true}));
    }
}
