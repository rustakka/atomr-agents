//! Document loaders. Each produces `Vec<Document>` from a source.

use std::path::PathBuf;

use async_trait::async_trait;
use atomr_agents_core::{AgentError, Result};
use atomr_agents_retriever::Document;

#[async_trait]
pub trait Loader: Send + Sync + 'static {
    async fn load(&self) -> Result<Vec<Document>>;
}

// --------------------------------------------------------------------
// TextLoader
// --------------------------------------------------------------------

pub struct TextLoader {
    pub paths: Vec<PathBuf>,
}

impl TextLoader {
    pub fn new(paths: impl IntoIterator<Item = impl Into<PathBuf>>) -> Self {
        Self { paths: paths.into_iter().map(Into::into).collect() }
    }
}

#[async_trait]
impl Loader for TextLoader {
    async fn load(&self) -> Result<Vec<Document>> {
        let mut out = Vec::with_capacity(self.paths.len());
        for p in &self.paths {
            let text = tokio::fs::read_to_string(p)
                .await
                .map_err(|e| AgentError::Internal(format!("read {}: {e}", p.display())))?;
            let id = p.to_string_lossy().to_string();
            let mut d = Document::new(id, text);
            d.metadata = serde_json::json!({"path": p.to_string_lossy()});
            out.push(d);
        }
        Ok(out)
    }
}

// --------------------------------------------------------------------
// MarkdownLoader — same as TextLoader but tags metadata.kind="markdown"
// --------------------------------------------------------------------

pub struct MarkdownLoader(pub TextLoader);

#[async_trait]
impl Loader for MarkdownLoader {
    async fn load(&self) -> Result<Vec<Document>> {
        let mut docs = self.0.load().await?;
        for d in &mut docs {
            if let serde_json::Value::Object(m) = &mut d.metadata {
                m.insert("kind".into(), serde_json::Value::String("markdown".into()));
            }
        }
        Ok(docs)
    }
}

// --------------------------------------------------------------------
// JsonLoader — array of {id, text, metadata?} → Documents.
// --------------------------------------------------------------------

pub struct JsonLoader {
    pub paths: Vec<PathBuf>,
}

impl JsonLoader {
    pub fn new(paths: impl IntoIterator<Item = impl Into<PathBuf>>) -> Self {
        Self { paths: paths.into_iter().map(Into::into).collect() }
    }
}

#[async_trait]
impl Loader for JsonLoader {
    async fn load(&self) -> Result<Vec<Document>> {
        let mut out = Vec::new();
        for p in &self.paths {
            let raw = tokio::fs::read_to_string(p)
                .await
                .map_err(|e| AgentError::Internal(format!("read {}: {e}", p.display())))?;
            let v: serde_json::Value = serde_json::from_str(&raw)
                .map_err(|e| AgentError::Internal(format!("parse {}: {e}", p.display())))?;
            let arr = v
                .as_array()
                .ok_or_else(|| AgentError::Internal(format!("{}: not a JSON array", p.display())))?;
            for entry in arr {
                let id = entry
                    .get("id")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let text = entry
                    .get("text")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let mut d = Document::new(id, text);
                d.metadata = entry.get("metadata").cloned().unwrap_or(serde_json::Value::Null);
                out.push(d);
            }
        }
        Ok(out)
    }
}

// --------------------------------------------------------------------
// CsvLoader — header row drives field names; produces one Document
// per row with metadata holding the row keys.
// --------------------------------------------------------------------

pub struct CsvLoader {
    pub paths: Vec<PathBuf>,
    pub text_field: String,
}

impl CsvLoader {
    pub fn new(paths: impl IntoIterator<Item = impl Into<PathBuf>>, text_field: impl Into<String>) -> Self {
        Self {
            paths: paths.into_iter().map(Into::into).collect(),
            text_field: text_field.into(),
        }
    }
}

#[async_trait]
impl Loader for CsvLoader {
    async fn load(&self) -> Result<Vec<Document>> {
        let mut out = Vec::new();
        for p in &self.paths {
            let raw = tokio::fs::read_to_string(p)
                .await
                .map_err(|e| AgentError::Internal(format!("read {}: {e}", p.display())))?;
            let mut lines = raw.lines();
            let Some(header) = lines.next() else { continue };
            let cols: Vec<&str> = header.split(',').collect();
            let text_idx = cols
                .iter()
                .position(|c| *c == self.text_field)
                .ok_or_else(|| AgentError::Internal(format!("text field {} missing", self.text_field)))?;
            for (i, line) in lines.enumerate() {
                let parts: Vec<&str> = line.split(',').collect();
                if parts.len() != cols.len() {
                    continue;
                }
                let mut metadata = serde_json::Map::new();
                for (c, v) in cols.iter().zip(parts.iter()) {
                    metadata.insert((*c).into(), serde_json::Value::String((*v).into()));
                }
                let d = Document {
                    id: format!("{}#{i}", p.display()),
                    text: parts[text_idx].to_string(),
                    metadata: serde_json::Value::Object(metadata),
                    score: 0.0,
                };
                out.push(d);
            }
        }
        Ok(out)
    }
}
