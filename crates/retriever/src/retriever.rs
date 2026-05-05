use async_trait::async_trait;
use atomr_agents_core::{CallCtx, Result, Value};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub metadata: Value,
    pub score: f32,
}

impl Document {
    pub fn new(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            metadata: Value::Null,
            score: 0.0,
        }
    }
}

#[async_trait]
pub trait Retriever: Send + Sync + 'static {
    async fn retrieve(&self, query: &str, ctx: &CallCtx) -> Result<Vec<Document>>;
}
