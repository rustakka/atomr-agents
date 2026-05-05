//! Pure-Rust BM25 over an in-memory corpus.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{CallCtx, Result};
use parking_lot::RwLock;

use crate::retriever::{Document, Retriever};

const K1: f32 = 1.5;
const B: f32 = 0.75;

struct Index {
    docs: Vec<Document>,
    df: HashMap<String, u32>,
    avg_len: f32,
}

pub struct Bm25Retriever {
    index: Arc<RwLock<Index>>,
    top_k: usize,
}

impl Bm25Retriever {
    pub fn new(top_k: usize) -> Self {
        Self {
            index: Arc::new(RwLock::new(Index { docs: Vec::new(), df: HashMap::new(), avg_len: 0.0 })),
            top_k,
        }
    }

    pub fn add(&self, doc: Document) {
        let mut idx = self.index.write();
        let tokens = tokenize(&doc.text);
        for t in dedup(&tokens) {
            *idx.df.entry(t).or_insert(0) += 1;
        }
        idx.docs.push(doc);
        let total: usize = idx.docs.iter().map(|d| tokenize(&d.text).len()).sum();
        idx.avg_len = if idx.docs.is_empty() { 0.0 } else { total as f32 / idx.docs.len() as f32 };
    }

    pub fn add_many(&self, docs: impl IntoIterator<Item = Document>) {
        for d in docs {
            self.add(d);
        }
    }
}

fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect()
}

fn dedup(tokens: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for t in tokens {
        if seen.insert(t.clone()) {
            out.push(t.clone());
        }
    }
    out
}

#[async_trait]
impl Retriever for Bm25Retriever {
    async fn retrieve(&self, query: &str, _ctx: &CallCtx) -> Result<Vec<Document>> {
        let idx = self.index.read();
        if idx.docs.is_empty() {
            return Ok(Vec::new());
        }
        let q_tokens = tokenize(query);
        let n = idx.docs.len() as f32;
        let mut scored: Vec<(usize, f32)> = idx
            .docs
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let tokens = tokenize(&d.text);
                let dl = tokens.len() as f32;
                let mut tf: HashMap<&str, u32> = HashMap::new();
                for t in &tokens {
                    *tf.entry(t.as_str()).or_insert(0) += 1;
                }
                let mut score = 0.0;
                for q in &q_tokens {
                    let f = *tf.get(q.as_str()).unwrap_or(&0) as f32;
                    if f == 0.0 {
                        continue;
                    }
                    let df = *idx.df.get(q).unwrap_or(&0) as f32;
                    let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
                    let denom = f + K1 * (1.0 - B + B * dl / idx.avg_len.max(1.0));
                    score += idf * (f * (K1 + 1.0)) / denom.max(1e-6);
                }
                (i, score)
            })
            .filter(|(_, s)| *s > 0.0)
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(self.top_k);
        Ok(scored
            .into_iter()
            .map(|(i, s)| {
                let mut d = idx.docs[i].clone();
                d.score = s;
                d
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_core::{IterationBudget, MoneyBudget, TimeBudget, TokenBudget};
    use std::time::Duration;

    fn ctx() -> CallCtx {
        CallCtx {
            agent_id: None,
            tokens: TokenBudget::new(1000),
            time: TimeBudget::new(Duration::from_secs(5)),
            money: MoneyBudget::from_usd(0.10),
            iterations: IterationBudget::new(5),
            trace: vec![],
        }
    }

    #[tokio::test]
    async fn bm25_ranks_relevant_doc_higher() {
        let r = Bm25Retriever::new(5);
        r.add(Document::new("d1", "Rust is a systems programming language"));
        r.add(Document::new("d2", "Python is great for data science"));
        r.add(Document::new("d3", "Cargo is the Rust build tool"));
        let hits = r.retrieve("rust cargo", &ctx()).await.unwrap();
        assert_eq!(hits[0].id, "d3");
    }
}
