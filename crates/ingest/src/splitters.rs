//! Text splitters.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::Result;
use atomr_agents_embed::Embedder;
use atomr_agents_retriever::Document;

pub trait Splitter: Send + Sync + 'static {
    fn split(&self, doc: &Document) -> Vec<Document>;

    fn split_all(&self, docs: &[Document]) -> Vec<Document> {
        let mut out = Vec::new();
        for d in docs {
            out.extend(self.split(d));
        }
        out
    }
}

// --------------------------------------------------------------------
// RecursiveCharacterSplitter
// --------------------------------------------------------------------

pub struct RecursiveCharacterSplitter {
    pub chunk_size: usize,
    pub overlap: usize,
    pub separators: Vec<String>,
}

impl RecursiveCharacterSplitter {
    pub fn new(chunk_size: usize, overlap: usize) -> Self {
        Self {
            chunk_size,
            overlap,
            separators: vec!["\n\n".into(), "\n".into(), ". ".into(), " ".into()],
        }
    }

    fn split_recursive(&self, text: &str, sep_idx: usize) -> Vec<String> {
        if text.chars().count() <= self.chunk_size {
            return vec![text.to_string()];
        }
        if sep_idx >= self.separators.len() {
            // Fall back to hard slicing.
            return text
                .as_bytes()
                .chunks(self.chunk_size.max(1))
                .map(|b| String::from_utf8_lossy(b).to_string())
                .collect();
        }
        let sep = &self.separators[sep_idx];
        let mut parts: Vec<String> = text.split(sep.as_str()).map(|s| s.to_string()).collect();
        // If splitting did nothing useful, recurse with next separator.
        if parts.len() == 1 {
            return self.split_recursive(text, sep_idx + 1);
        }
        // Greedy merge until we exceed chunk_size, then start a new chunk.
        let mut chunks = Vec::new();
        let mut current = String::new();
        for p in parts.drain(..) {
            if current.is_empty() {
                current = p;
            } else if current.chars().count() + sep.chars().count() + p.chars().count() <= self.chunk_size {
                current.push_str(sep);
                current.push_str(&p);
            } else {
                chunks.push(std::mem::take(&mut current));
                current = p;
            }
        }
        if !current.is_empty() {
            chunks.push(current);
        }
        // Recurse into oversized chunks with the next separator.
        let mut out = Vec::new();
        for c in chunks {
            if c.chars().count() > self.chunk_size {
                out.extend(self.split_recursive(&c, sep_idx + 1));
            } else {
                out.push(c);
            }
        }
        // Apply overlap by stitching the last `overlap` chars onto each.
        if self.overlap > 0 {
            let mut with_overlap: Vec<String> = Vec::with_capacity(out.len());
            for (i, c) in out.iter().enumerate() {
                if i == 0 {
                    with_overlap.push(c.clone());
                } else {
                    let prev: String = out[i - 1]
                        .chars()
                        .rev()
                        .take(self.overlap)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    with_overlap.push(format!("{prev}{c}"));
                }
            }
            return with_overlap;
        }
        out
    }
}

impl Splitter for RecursiveCharacterSplitter {
    fn split(&self, doc: &Document) -> Vec<Document> {
        let chunks = self.split_recursive(&doc.text, 0);
        chunks
            .into_iter()
            .enumerate()
            .map(|(i, t)| Document {
                id: format!("{}#chunk{i}", doc.id),
                text: t,
                metadata: doc.metadata.clone(),
                score: 0.0,
            })
            .collect()
    }
}

// --------------------------------------------------------------------
// MarkdownHeaderSplitter — split at # / ## / ### markers; one chunk
// per section.
// --------------------------------------------------------------------

pub struct MarkdownHeaderSplitter {
    pub max_level: u8,
}

impl Default for MarkdownHeaderSplitter {
    fn default() -> Self {
        Self { max_level: 3 }
    }
}

impl Splitter for MarkdownHeaderSplitter {
    fn split(&self, doc: &Document) -> Vec<Document> {
        let mut sections: Vec<(String, String)> = Vec::new();
        let mut current_header = String::new();
        let mut current_body = String::new();
        for line in doc.text.lines() {
            let trimmed = line.trim_start();
            let level = trimmed.chars().take_while(|c| *c == '#').count() as u8;
            if level > 0 && level <= self.max_level && trimmed.chars().nth(level as usize) == Some(' ') {
                if !current_body.is_empty() || !current_header.is_empty() {
                    sections.push((current_header.clone(), std::mem::take(&mut current_body)));
                }
                current_header = trimmed.trim_start_matches('#').trim().to_string();
            } else {
                current_body.push_str(line);
                current_body.push('\n');
            }
        }
        if !current_body.is_empty() || !current_header.is_empty() {
            sections.push((current_header, current_body));
        }
        sections
            .into_iter()
            .enumerate()
            .map(|(i, (header, body))| Document {
                id: format!("{}#section{i}", doc.id),
                text: if header.is_empty() {
                    body
                } else {
                    format!("# {header}\n{body}")
                },
                metadata: doc.metadata.clone(),
                score: 0.0,
            })
            .collect()
    }
}

// --------------------------------------------------------------------
// CodeSplitter — split at top-level fn/struct/class boundaries per
// language. v0 ships a regex-style heuristic per known language.
// --------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeLang {
    Rust,
    Python,
    Js,
}

pub struct CodeSplitter {
    pub lang: CodeLang,
}

impl Splitter for CodeSplitter {
    fn split(&self, doc: &Document) -> Vec<Document> {
        let starters: &[&str] = match self.lang {
            CodeLang::Rust => &["fn ", "pub fn ", "struct ", "pub struct ", "impl ", "trait "],
            CodeLang::Python => &["def ", "class "],
            CodeLang::Js => &["function ", "class "],
        };
        let mut chunks = Vec::new();
        let mut buf = String::new();
        for line in doc.text.lines() {
            let starts = starters.iter().any(|s| line.trim_start().starts_with(s));
            if starts && !buf.is_empty() {
                chunks.push(std::mem::take(&mut buf));
            }
            buf.push_str(line);
            buf.push('\n');
        }
        if !buf.is_empty() {
            chunks.push(buf);
        }
        chunks
            .into_iter()
            .enumerate()
            .map(|(i, t)| Document {
                id: format!("{}#code{i}", doc.id),
                text: t,
                metadata: doc.metadata.clone(),
                score: 0.0,
            })
            .collect()
    }
}

// --------------------------------------------------------------------
// TokenSplitter — counts whitespace-separated tokens. Real impl
// would use a tokenizer; this is a budget-friendly approximation.
// --------------------------------------------------------------------

pub struct TokenSplitter {
    pub max_tokens: usize,
    pub overlap_tokens: usize,
}

impl Splitter for TokenSplitter {
    fn split(&self, doc: &Document) -> Vec<Document> {
        let toks: Vec<&str> = doc.text.split_whitespace().collect();
        let step = self.max_tokens.saturating_sub(self.overlap_tokens).max(1);
        let mut out = Vec::new();
        let mut start = 0;
        let mut i = 0;
        while start < toks.len() {
            let end = (start + self.max_tokens).min(toks.len());
            let chunk = toks[start..end].join(" ");
            out.push(Document {
                id: format!("{}#tk{i}", doc.id),
                text: chunk,
                metadata: doc.metadata.clone(),
                score: 0.0,
            });
            i += 1;
            if end == toks.len() {
                break;
            }
            start += step;
        }
        out
    }
}

// --------------------------------------------------------------------
// SemanticSplitter — embed sentences, break at low-similarity
// boundaries. Sync interface (Splitter is sync); we run the embed
// asynchronously up front via `precompute`.
// --------------------------------------------------------------------

pub struct SemanticSplitter {
    pub embedder: Arc<dyn Embedder>,
    pub similarity_threshold: f32,
    pub max_chunk_chars: usize,
}

impl SemanticSplitter {
    pub async fn split_async(&self, doc: &Document) -> Result<Vec<Document>> {
        let sentences: Vec<String> = doc
            .text
            .split('.')
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string())
            .collect();
        if sentences.len() <= 1 {
            return Ok(vec![doc.clone()]);
        }
        let embeddings = self.embedder.embed_batch(&sentences).await?;
        let mut chunks: Vec<String> = Vec::new();
        let mut current = sentences[0].clone();
        for i in 1..sentences.len() {
            let sim = cosine(&embeddings[i - 1], &embeddings[i]);
            let too_long = current.len() >= self.max_chunk_chars;
            if sim < self.similarity_threshold || too_long {
                chunks.push(std::mem::take(&mut current));
                current = sentences[i].clone();
            } else {
                current.push_str(". ");
                current.push_str(&sentences[i]);
            }
        }
        if !current.is_empty() {
            chunks.push(current);
        }
        Ok(chunks
            .into_iter()
            .enumerate()
            .map(|(i, t)| Document {
                id: format!("{}#sem{i}", doc.id),
                text: t,
                metadata: doc.metadata.clone(),
                score: 0.0,
            })
            .collect())
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

// silence the unused-trait-method warning when only `split_async` is used.
#[allow(dead_code)]
fn _splitter_in_scope<T: Splitter>(_t: &T) {}

// SemanticSplitter doesn't impl `Splitter` because it's async-only.
#[async_trait]
pub trait AsyncSplitter: Send + Sync + 'static {
    async fn split(&self, doc: &Document) -> Result<Vec<Document>>;
}

#[async_trait]
impl AsyncSplitter for SemanticSplitter {
    async fn split(&self, doc: &Document) -> Result<Vec<Document>> {
        self.split_async(doc).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_embed::MockEmbedder;

    #[test]
    fn recursive_splitter_chunks_long_text() {
        let s = RecursiveCharacterSplitter::new(40, 0);
        let d = Document::new(
            "d",
            "Paragraph one is here.\n\nParagraph two is also here, with more words to push it over.",
        );
        let chunks = s.split(&d);
        assert!(chunks.len() >= 2);
    }

    #[test]
    fn markdown_header_splitter_creates_per_section_chunks() {
        let s = MarkdownHeaderSplitter::default();
        let d = Document::new("d", "# Title\nintro\n## Sub one\nbody A\n## Sub two\nbody B\n");
        let chunks = s.split(&d);
        assert!(chunks.len() >= 2);
    }

    #[test]
    fn code_splitter_breaks_at_fn_boundaries() {
        let s = CodeSplitter { lang: CodeLang::Rust };
        let d = Document::new("d", "fn a() { }\nfn b() { }\n");
        let chunks = s.split(&d);
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn token_splitter_overlaps() {
        let s = TokenSplitter {
            max_tokens: 5,
            overlap_tokens: 2,
        };
        let d = Document::new("d", "a b c d e f g h i j");
        let chunks = s.split(&d);
        assert!(chunks.len() >= 2);
        assert_eq!(chunks[0].text, "a b c d e");
    }

    #[tokio::test]
    async fn semantic_splitter_splits_at_topic_change() {
        let emb: Arc<dyn Embedder> = Arc::new(MockEmbedder::new(8));
        let s = SemanticSplitter {
            embedder: emb,
            similarity_threshold: 0.99,
            max_chunk_chars: 1_000,
        };
        let d = Document::new(
            "d",
            "rust is a language. rust has cargo. python is great. python has numpy",
        );
        let chunks = s.split_async(&d).await.unwrap();
        assert!(chunks.len() >= 2);
    }
}
