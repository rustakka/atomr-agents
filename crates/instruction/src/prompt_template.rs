//! ChatPromptTemplate + few-shot.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentError, MessageRole, Result, Value};
use atomr_agents_embed::Embedder;
use serde::{Deserialize, Serialize};

/// Simple `{var}` interpolation. Missing vars resolve to empty string.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StringTemplate(pub String);

impl StringTemplate {
    pub fn render(&self, vars: &HashMap<String, Value>) -> String {
        let mut out = self.0.clone();
        for (k, v) in vars {
            let placeholder = format!("{{{k}}}");
            let replacement = match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            out = out.replace(&placeholder, &replacement);
        }
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageTemplate {
    System(StringTemplate),
    User(StringTemplate),
    Assistant(StringTemplate),
    /// Insert one or more messages from `vars[key]` (must be an array).
    Placeholder {
        key: String,
    },
}

/// MessagesPlaceholder analogue — same shape as the `Placeholder`
/// variant, exported under the LangChain-familiar name.
pub type MessagesPlaceholder = MessageTemplate;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderedMessage {
    pub role: MessageRole,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatPromptTemplate {
    pub messages: Vec<MessageTemplate>,
    #[serde(default)]
    pub partial: HashMap<String, Value>,
}

impl ChatPromptTemplate {
    pub fn builder() -> ChatPromptTemplateBuilder {
        ChatPromptTemplateBuilder {
            messages: Vec::new(),
            partial: HashMap::new(),
        }
    }

    pub fn partial(mut self, key: impl Into<String>, val: Value) -> Self {
        self.partial.insert(key.into(), val);
        self
    }

    pub fn render(&self, vars: &HashMap<String, Value>) -> Result<Vec<RenderedMessage>> {
        let mut merged: HashMap<String, Value> = self.partial.clone();
        for (k, v) in vars {
            merged.insert(k.clone(), v.clone());
        }
        let mut out = Vec::new();
        for m in &self.messages {
            match m {
                MessageTemplate::System(t) => out.push(RenderedMessage {
                    role: MessageRole::System,
                    content: t.render(&merged),
                }),
                MessageTemplate::User(t) => out.push(RenderedMessage {
                    role: MessageRole::User,
                    content: t.render(&merged),
                }),
                MessageTemplate::Assistant(t) => out.push(RenderedMessage {
                    role: MessageRole::Assistant,
                    content: t.render(&merged),
                }),
                MessageTemplate::Placeholder { key } => {
                    let v = merged.get(key).cloned().unwrap_or(Value::Null);
                    let arr = v.as_array().cloned().unwrap_or_default();
                    for entry in arr {
                        let role =
                            role_from_str(entry.get("role").and_then(|x| x.as_str()).unwrap_or("user"));
                        let content = entry
                            .get("content")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string();
                        out.push(RenderedMessage { role, content });
                    }
                }
            }
        }
        Ok(out)
    }
}

fn role_from_str(s: &str) -> MessageRole {
    match s {
        "system" => MessageRole::System,
        "assistant" => MessageRole::Assistant,
        "tool" => MessageRole::Tool,
        _ => MessageRole::User,
    }
}

pub struct ChatPromptTemplateBuilder {
    messages: Vec<MessageTemplate>,
    partial: HashMap<String, Value>,
}

impl ChatPromptTemplateBuilder {
    pub fn system(mut self, text: impl Into<String>) -> Self {
        self.messages
            .push(MessageTemplate::System(StringTemplate(text.into())));
        self
    }
    pub fn user(mut self, text: impl Into<String>) -> Self {
        self.messages
            .push(MessageTemplate::User(StringTemplate(text.into())));
        self
    }
    pub fn assistant(mut self, text: impl Into<String>) -> Self {
        self.messages
            .push(MessageTemplate::Assistant(StringTemplate(text.into())));
        self
    }
    pub fn placeholder(mut self, key: impl Into<String>) -> Self {
        self.messages
            .push(MessageTemplate::Placeholder { key: key.into() });
        self
    }
    pub fn partial(mut self, key: impl Into<String>, val: Value) -> Self {
        self.partial.insert(key.into(), val);
        self
    }
    pub fn build(self) -> ChatPromptTemplate {
        ChatPromptTemplate {
            messages: self.messages,
            partial: self.partial,
        }
    }
}

// --------------------------------------------------------------------
// Few-shot
// --------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Example {
    pub vars: HashMap<String, Value>,
    /// Token-count estimate; used by `LengthBasedSelector`.
    #[serde(default)]
    pub estimated_tokens: u32,
    /// Free-form text used by semantic selection (typically the user
    /// prompt of the example).
    #[serde(default)]
    pub query_text: String,
}

#[async_trait]
pub trait ExampleSelector: Send + Sync + 'static {
    async fn select(&self, vars: &HashMap<String, Value>) -> Result<Vec<Example>>;
}

/// Length-based: greedily picks examples in order until `max_tokens`
/// is exceeded.
pub struct LengthBasedSelector {
    pub examples: Vec<Example>,
    pub max_tokens: u32,
}

#[async_trait]
impl ExampleSelector for LengthBasedSelector {
    async fn select(&self, _vars: &HashMap<String, Value>) -> Result<Vec<Example>> {
        let mut acc = 0u32;
        let mut out = Vec::new();
        for e in &self.examples {
            if acc + e.estimated_tokens > self.max_tokens {
                break;
            }
            acc += e.estimated_tokens;
            out.push(e.clone());
        }
        Ok(out)
    }
}

/// Semantic-similarity selector: picks the top-k examples whose
/// `query_text` is closest to `vars[query_key]`.
pub struct SemanticSimilaritySelector {
    pub examples: Vec<Example>,
    pub embedder: Arc<dyn Embedder>,
    pub query_key: String,
    pub top_k: usize,
}

#[async_trait]
impl ExampleSelector for SemanticSimilaritySelector {
    async fn select(&self, vars: &HashMap<String, Value>) -> Result<Vec<Example>> {
        let q = vars
            .get(&self.query_key)
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Internal(format!("missing var '{}'", self.query_key)))?;
        let qv = self.embedder.embed(q).await?;
        let mut scored: Vec<(f32, Example)> = Vec::with_capacity(self.examples.len());
        for e in &self.examples {
            let v = self.embedder.embed(&e.query_text).await?;
            scored.push((cosine(&qv, &v), e.clone()));
        }
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(self.top_k);
        Ok(scored.into_iter().map(|(_, e)| e).collect())
    }
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

pub struct FewShotChatTemplate {
    pub formatter: ChatPromptTemplate,
    pub selector: Arc<dyn ExampleSelector>,
    /// Sub-template used to render each picked example. Per-example
    /// vars come from `Example.vars`.
    pub example_template: ChatPromptTemplate,
}

impl FewShotChatTemplate {
    pub fn new(
        formatter: ChatPromptTemplate,
        selector: Arc<dyn ExampleSelector>,
        example_template: ChatPromptTemplate,
    ) -> Self {
        Self {
            formatter,
            selector,
            example_template,
        }
    }

    pub async fn render(&self, vars: &HashMap<String, Value>) -> Result<Vec<RenderedMessage>> {
        let examples = self.selector.select(vars).await?;
        let mut all = Vec::new();
        for ex in &examples {
            let rendered = self.example_template.render(&ex.vars)?;
            all.extend(rendered);
        }
        all.extend(self.formatter.render(vars)?);
        Ok(all)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_embed::MockEmbedder;

    fn vars(pairs: &[(&str, Value)]) -> HashMap<String, Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn chat_template_renders_with_vars() {
        let t = ChatPromptTemplate::builder()
            .system("You are a {role}.")
            .user("Help me with {task}.")
            .build();
        let r = t
            .render(&vars(&[
                ("role", Value::String("researcher".into())),
                ("task", Value::String("RAG".into())),
            ]))
            .unwrap();
        assert_eq!(r.len(), 2);
        assert!(r[0].content.contains("researcher"));
        assert!(r[1].content.contains("RAG"));
    }

    #[test]
    fn placeholder_inserts_history() {
        let t = ChatPromptTemplate::builder()
            .system("ok")
            .placeholder("history")
            .user("{q}")
            .build();
        let history = serde_json::json!([
            {"role": "user", "content": "first"},
            {"role": "assistant", "content": "second"},
        ]);
        let r = t
            .render(&vars(&[
                ("history", history),
                ("q", Value::String("third".into())),
            ]))
            .unwrap();
        assert_eq!(r.len(), 4);
        assert_eq!(r[3].content, "third");
    }

    #[tokio::test]
    async fn length_based_selector_truncates_at_budget() {
        let s = LengthBasedSelector {
            examples: vec![
                Example {
                    vars: HashMap::new(),
                    estimated_tokens: 50,
                    query_text: "a".into(),
                },
                Example {
                    vars: HashMap::new(),
                    estimated_tokens: 40,
                    query_text: "b".into(),
                },
                Example {
                    vars: HashMap::new(),
                    estimated_tokens: 30,
                    query_text: "c".into(),
                },
            ],
            max_tokens: 80,
        };
        let chosen = s.select(&HashMap::new()).await.unwrap();
        assert_eq!(chosen.len(), 1);
    }

    #[tokio::test]
    async fn semantic_selector_picks_closest() {
        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder::new(8));
        let s = SemanticSimilaritySelector {
            examples: vec![
                Example {
                    vars: vars(&[("input", Value::String("rust".into()))]),
                    estimated_tokens: 10,
                    query_text: "rust".into(),
                },
                Example {
                    vars: vars(&[("input", Value::String("python".into()))]),
                    estimated_tokens: 10,
                    query_text: "python".into(),
                },
            ],
            embedder,
            query_key: "input".into(),
            top_k: 1,
        };
        let chosen = s
            .select(&vars(&[("input", Value::String("rust".into()))]))
            .await
            .unwrap();
        assert_eq!(chosen.len(), 1);
        assert_eq!(chosen[0].query_text, "rust");
    }

    #[tokio::test]
    async fn few_shot_renders_examples_then_formatter() {
        let formatter = ChatPromptTemplate::builder().user("Now answer: {q}").build();
        let example_tmpl = ChatPromptTemplate::builder()
            .user("Q: {q}")
            .assistant("A: {a}")
            .build();
        let selector: Arc<dyn ExampleSelector> = Arc::new(LengthBasedSelector {
            examples: vec![Example {
                vars: vars(&[
                    ("q", Value::String("2+2?".into())),
                    ("a", Value::String("4".into())),
                ]),
                estimated_tokens: 10,
                query_text: "math".into(),
            }],
            max_tokens: 100,
        });
        let few_shot = FewShotChatTemplate::new(formatter, selector, example_tmpl);
        let r = few_shot
            .render(&vars(&[("q", Value::String("3+4?".into()))]))
            .await
            .unwrap();
        assert_eq!(r.len(), 3);
        assert!(r[0].content.contains("2+2?"));
        assert!(r[1].content.contains("4"));
        assert!(r[2].content.contains("3+4?"));
    }
}
