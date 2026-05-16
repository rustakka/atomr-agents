//! Cognition actor — turns a transcribed utterance into a structured
//! agent reply envelope (text + emotion delta + optional gesture).
//!
//! The implementation calls atomr-infer's [`ModelRunner`] via a thin
//! [`AvatarInferenceClient`] trait the caller supplies. That keeps this
//! crate independent of `crates/agent` while still routing every model
//! call through atomr-infer — exactly what the project directive
//! requires.

use async_trait::async_trait;
use atomr_agents_avatar_core::{AvatarError, EmotionDelta, Result};
use atomr_infer_core::batch::{ExecuteBatch, Message, MessageContent, Role, SamplingParams};
use serde::{Deserialize, Serialize};

/// Cognition configuration — persona prompt, model, decoding params.
#[derive(Debug, Clone)]
pub struct CognitionConfig {
    /// Persona / system prompt for the agent. We append a fixed
    /// JSON-envelope instruction so the response can be parsed.
    pub persona_prompt: String,
    /// Model identifier passed through to atomr-infer (e.g.
    /// `"claude-opus-4-7"`, `"gpt-4o"`, `"local-llama-3.1-8b"`).
    pub model: String,
    /// Sampling params forwarded to the runtime.
    pub sampling: SamplingParams,
    /// Maximum input tokens hint for atomr-infer's rate limiter.
    pub estimated_tokens: u32,
}

impl Default for CognitionConfig {
    fn default() -> Self {
        Self {
            persona_prompt:
                "You are a warm, engaging conversational avatar. Keep responses concise (≤2 sentences)."
                    .to_string(),
            model: "claude-haiku-4-5-20251001".to_string(),
            sampling: SamplingParams {
                temperature: Some(0.7),
                max_tokens: Some(160),
                ..Default::default()
            },
            estimated_tokens: 256,
        }
    }
}

/// Optional gesture cue the cognition layer can attach to a reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GestureHint {
    Nod,
    Shake,
    Shrug,
    Wave,
    Point,
    Idle,
}

/// Structured envelope the agent emits per turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentIntentPacket {
    pub response_text: String,
    #[serde(default)]
    pub emotion_delta: EmotionDelta,
    #[serde(default)]
    pub gesture: Option<GestureHint>,
}

impl AgentIntentPacket {
    /// Construct a neutral intent — handy as a fallback when the
    /// model returns plain prose without the JSON envelope.
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            response_text: text.into(),
            emotion_delta: EmotionDelta::default(),
            gesture: None,
        }
    }
}

/// Trait the harness uses to call into atomr-infer. Callers wrap a
/// `ModelRunner` (local) or an Anthropic/OpenAI HTTP client behind
/// this — kept narrow so atomr-infer remains the single model-call
/// surface in the framework.
#[async_trait]
pub trait AvatarInferenceClient: Send + Sync + 'static {
    /// Run a single inference. Returns the assembled text from the
    /// underlying token stream (the harness parses JSON itself).
    async fn complete(&self, batch: ExecuteBatch) -> Result<String>;
}

/// The cognition actor. Stateless per-call — the harness owns it
/// behind an `Arc` and re-uses it across turns.
pub struct CognitionActor {
    client: std::sync::Arc<dyn AvatarInferenceClient>,
    cfg: CognitionConfig,
}

impl CognitionActor {
    pub fn new(
        client: std::sync::Arc<dyn AvatarInferenceClient>,
        cfg: CognitionConfig,
    ) -> Self {
        Self { client, cfg }
    }

    /// Drive one cognition turn against the given user utterance.
    /// Returns a parsed [`AgentIntentPacket`] (falls back to plain
    /// text if the model didn't emit a clean JSON envelope).
    pub async fn handle_utterance(&self, user_text: &str) -> Result<AgentIntentPacket> {
        let system = format!(
            "{persona}\n\n\
             Respond ONLY in this JSON envelope (no preamble, no markdown fences):\n\
             {{\"response_text\": \"…\", \"emotion_delta\": {{\"valence\": -1..=1, \"arousal\": 0..=1, \"anger\": 0..=1, \"surprise\": 0..=1, \"tension\": 0..=1}}, \"gesture\": \"nod|shake|shrug|wave|point|idle|null\"}}",
            persona = self.cfg.persona_prompt,
        );

        let batch = ExecuteBatch {
            request_id: uuid_v4(),
            model: self.cfg.model.clone(),
            messages: vec![
                Message {
                    role: Role::System,
                    content: MessageContent::Text(system),
                },
                Message {
                    role: Role::User,
                    content: MessageContent::Text(user_text.to_string()),
                },
            ],
            sampling: self.cfg.sampling.clone(),
            stream: false,
            estimated_tokens: self.cfg.estimated_tokens,
        };

        let text = self
            .client
            .complete(batch)
            .await
            .map_err(|e| AvatarError::cognition(e.to_string()))?;

        Ok(parse_intent(&text))
    }
}

fn parse_intent(text: &str) -> AgentIntentPacket {
    // Trim Markdown fencing the model may have introduced anyway.
    let cleaned = strip_code_fences(text.trim());
    match serde_json::from_str::<AgentIntentPacket>(cleaned) {
        Ok(packet) => packet,
        Err(_) => AgentIntentPacket::plain(text.to_string()),
    }
}

fn strip_code_fences(s: &str) -> &str {
    let mut s = s;
    if let Some(rest) = s.strip_prefix("```json") {
        s = rest;
    } else if let Some(rest) = s.strip_prefix("```") {
        s = rest;
    }
    if let Some(stripped) = s.strip_suffix("```") {
        s = stripped;
    }
    s.trim()
}

fn uuid_v4() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    // Lightweight monotonic id; we don't depend on the `uuid` crate
    // here because correlation only needs to be unique per process.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("avatar-{nanos:x}-{n:x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    struct StubClient {
        responses: Mutex<Vec<String>>,
        seen: Mutex<Vec<ExecuteBatch>>,
    }

    #[async_trait]
    impl AvatarInferenceClient for StubClient {
        async fn complete(&self, batch: ExecuteBatch) -> Result<String> {
            self.seen.lock().await.push(batch);
            let mut r = self.responses.lock().await;
            Ok(r.remove(0))
        }
    }

    #[tokio::test]
    async fn parses_json_envelope() {
        let stub = Arc::new(StubClient {
            responses: Mutex::new(vec![r#"{"response_text":"Hi!","emotion_delta":{"valence":0.7,"arousal":0.3,"anger":0.0,"surprise":0.1,"tension":0.0},"gesture":"wave"}"#.to_string()]),
            seen: Mutex::new(Vec::new()),
        });
        let cog = CognitionActor::new(stub.clone(), CognitionConfig::default());
        let intent = cog.handle_utterance("hello").await.unwrap();
        assert_eq!(intent.response_text, "Hi!");
        assert_eq!(intent.gesture, Some(GestureHint::Wave));
        assert!((intent.emotion_delta.valence - 0.7).abs() < 1e-6);
    }

    #[tokio::test]
    async fn unfenced_markdown_is_stripped() {
        let stub = Arc::new(StubClient {
            responses: Mutex::new(vec!["```json\n{\"response_text\":\"ok\"}\n```".to_string()]),
            seen: Mutex::new(Vec::new()),
        });
        let cog = CognitionActor::new(stub.clone(), CognitionConfig::default());
        let intent = cog.handle_utterance("hi").await.unwrap();
        assert_eq!(intent.response_text, "ok");
    }

    #[tokio::test]
    async fn plain_text_fallback() {
        let stub = Arc::new(StubClient {
            responses: Mutex::new(vec!["Hello there.".to_string()]),
            seen: Mutex::new(Vec::new()),
        });
        let cog = CognitionActor::new(stub.clone(), CognitionConfig::default());
        let intent = cog.handle_utterance("hi").await.unwrap();
        assert_eq!(intent.response_text, "Hello there.");
        assert_eq!(intent.gesture, None);
    }

    #[tokio::test]
    async fn batch_carries_system_and_user_messages() {
        let stub = Arc::new(StubClient {
            responses: Mutex::new(vec![r#"{"response_text":"ok"}"#.to_string()]),
            seen: Mutex::new(Vec::new()),
        });
        let cog = CognitionActor::new(stub.clone(), CognitionConfig::default());
        cog.handle_utterance("hi").await.unwrap();
        let seen = stub.seen.lock().await;
        let batch = seen.last().unwrap();
        assert_eq!(batch.messages.len(), 2);
        assert!(matches!(batch.messages[0].role, Role::System));
        assert!(matches!(batch.messages[1].role, Role::User));
    }
}
