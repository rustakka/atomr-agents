//! Tracer trait + stock implementations.
//!
//! A `Tracer` consumes the `RunTree` produced by `RunTreeBuilder` and
//! emits it somewhere — stdout, a JSONL file, or LangSmith. Tracers
//! are themselves `EventBus` subscribers; you wire them once at
//! startup and they receive everything emitted thereafter.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{EventEnvelope, Result};
use parking_lot::Mutex;
use serde::Serialize;

use crate::run_tree::{RunNode, RunTreeBuilder};

/// Side channel a tracer writes to. We keep this trait so unit tests
/// can capture output without touching the filesystem or network.
#[async_trait]
pub trait TracerSink: Send + Sync + 'static {
    async fn emit(&self, payload: &str) -> Result<()>;
}

/// Tracers wrap a `RunTreeBuilder` plus a `TracerSink` and flush
/// either eagerly (per event) or on demand.
#[async_trait]
pub trait Tracer: Send + Sync + 'static {
    /// Called per event. Default no-op; eager tracers override.
    async fn on_event(&self, _env: &EventEnvelope) -> Result<()> {
        Ok(())
    }
    /// Called by the user once the run is done. Streams roots out.
    async fn flush(&self) -> Result<()>;
}

// --------------------------------------------------------------------
// StdoutTracer
// --------------------------------------------------------------------

pub struct StdoutTracer {
    pub builder: Arc<RunTreeBuilder>,
}

impl StdoutTracer {
    pub fn new(builder: Arc<RunTreeBuilder>) -> Self {
        Self { builder }
    }
}

#[async_trait]
impl Tracer for StdoutTracer {
    async fn flush(&self) -> Result<()> {
        for root in self.builder.roots() {
            print_node(&self.builder, &root, 0);
        }
        Ok(())
    }
}

fn print_node(builder: &RunTreeBuilder, node: &RunNode, depth: usize) {
    let indent = "  ".repeat(depth);
    let elapsed = node.elapsed_ms().unwrap_or(0);
    println!(
        "{indent}- {} [{:?}] {} ms",
        node.name, node.kind, elapsed
    );
    for child_id in &node.children {
        if let Some(child) = builder.get(child_id) {
            print_node(builder, &child, depth + 1);
        }
    }
}

// --------------------------------------------------------------------
// JsonlTracer
// --------------------------------------------------------------------

pub struct JsonlTracer {
    pub builder: Arc<RunTreeBuilder>,
    sink: Arc<dyn TracerSink>,
}

impl JsonlTracer {
    pub fn new(builder: Arc<RunTreeBuilder>, sink: Arc<dyn TracerSink>) -> Self {
        Self { builder, sink }
    }

    /// Convenience constructor backed by an in-memory buffer (for tests).
    pub fn in_memory(builder: Arc<RunTreeBuilder>) -> (Self, Arc<MemorySink>) {
        let sink = Arc::new(MemorySink::default());
        (Self::new(builder, sink.clone()), sink)
    }
}

#[async_trait]
impl Tracer for JsonlTracer {
    async fn flush(&self) -> Result<()> {
        for root in self.builder.roots() {
            let line = serde_json::to_string(&root)
                .map_err(|e| atomr_agents_core::AgentError::Internal(e.to_string()))?;
            self.sink.emit(&line).await?;
            // Recurse so children are emitted too (one node per line).
            emit_descendants(&self.builder, &root, &self.sink).await?;
        }
        Ok(())
    }
}

async fn emit_descendants(
    builder: &RunTreeBuilder,
    node: &RunNode,
    sink: &Arc<dyn TracerSink>,
) -> Result<()> {
    for cid in &node.children {
        if let Some(child) = builder.get(cid) {
            let line = serde_json::to_string(&child)
                .map_err(|e| atomr_agents_core::AgentError::Internal(e.to_string()))?;
            sink.emit(&line).await?;
            // Manually recurse without async recursion (Rust async fn
            // recursion needs boxing).
            let mut stack: Vec<RunNode> = child.children.iter().filter_map(|c| builder.get(c)).collect();
            while let Some(n) = stack.pop() {
                let l = serde_json::to_string(&n)
                    .map_err(|e| atomr_agents_core::AgentError::Internal(e.to_string()))?;
                sink.emit(&l).await?;
                for cc in &n.children {
                    if let Some(grand) = builder.get(cc) {
                        stack.push(grand);
                    }
                }
            }
        }
    }
    Ok(())
}

// --------------------------------------------------------------------
// LangSmithTracer
// --------------------------------------------------------------------

/// Minimal LangSmith-shaped run record. Real LangSmith ingestion
/// expects a richer schema; this matches the most common fields and
/// is sufficient for offline import + integration testing against a
/// mock HTTP server.
#[derive(Debug, Serialize)]
pub struct LangSmithRunRecord<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub run_type: &'a str,
    pub start_time_ms: i64,
    pub end_time_ms: Option<i64>,
    pub parent_run_id: Option<&'a str>,
    pub tags: &'a [String],
    pub error: Option<&'a str>,
}

pub struct LangSmithTracer {
    pub builder: Arc<RunTreeBuilder>,
    pub project: String,
    sink: Arc<dyn TracerSink>,
}

impl LangSmithTracer {
    pub fn new(
        builder: Arc<RunTreeBuilder>,
        project: impl Into<String>,
        sink: Arc<dyn TracerSink>,
    ) -> Self {
        Self { builder, project: project.into(), sink }
    }

    pub fn in_memory(
        builder: Arc<RunTreeBuilder>,
        project: impl Into<String>,
    ) -> (Self, Arc<MemorySink>) {
        let sink = Arc::new(MemorySink::default());
        (Self::new(builder, project, sink.clone()), sink)
    }
}

#[async_trait]
impl Tracer for LangSmithTracer {
    async fn flush(&self) -> Result<()> {
        let snapshot = self.builder.snapshot();
        for node in snapshot.values() {
            let kind = match node.kind {
                crate::run_tree::RunKind::Agent => "chain",
                crate::run_tree::RunKind::Llm => "llm",
                crate::run_tree::RunKind::Tool => "tool",
                crate::run_tree::RunKind::Retriever => "retriever",
                crate::run_tree::RunKind::Parser => "parser",
                crate::run_tree::RunKind::Workflow => "chain",
                crate::run_tree::RunKind::Harness => "chain",
                crate::run_tree::RunKind::Chain => "chain",
                crate::run_tree::RunKind::Other => "chain",
            };
            let parent_str = node.parent_run_id.as_ref().map(|p| p.as_str());
            let record = LangSmithRunRecord {
                id: node.run_id.as_str(),
                name: &node.name,
                run_type: kind,
                start_time_ms: node.started_at_ms,
                end_time_ms: node.ended_at_ms,
                parent_run_id: parent_str,
                tags: &node.tags,
                error: node.error.as_deref(),
            };
            let mut json = serde_json::to_value(&record)
                .map_err(|e| atomr_agents_core::AgentError::Internal(e.to_string()))?;
            json["project"] = serde_json::Value::String(self.project.clone());
            let line = json.to_string();
            self.sink.emit(&line).await?;
        }
        Ok(())
    }
}

// --------------------------------------------------------------------
// MemorySink — testing helper, also used by JsonlTracer::in_memory.
// --------------------------------------------------------------------

#[derive(Default)]
pub struct MemorySink {
    pub lines: Mutex<Vec<String>>,
}

#[async_trait]
impl TracerSink for MemorySink {
    async fn emit(&self, payload: &str) -> Result<()> {
        self.lines.lock().push(payload.to_string());
        Ok(())
    }
}

// --------------------------------------------------------------------
// FileSink — writes each line + newline to a file.
// --------------------------------------------------------------------

#[allow(dead_code)]
pub struct FileSink {
    path: PathBuf,
}

impl FileSink {
    #[allow(dead_code)]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[async_trait]
impl TracerSink for FileSink {
    async fn emit(&self, payload: &str) -> Result<()> {
        use tokio::io::AsyncWriteExt;
        let mut f = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await
            .map_err(|e| atomr_agents_core::AgentError::Internal(e.to_string()))?;
        f.write_all(payload.as_bytes())
            .await
            .map_err(|e| atomr_agents_core::AgentError::Internal(e.to_string()))?;
        f.write_all(b"\n")
            .await
            .map_err(|e| atomr_agents_core::AgentError::Internal(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EventBus;
    use atomr_agents_core::{AgentId, Event, RunId, ToolId};

    #[tokio::test]
    async fn jsonl_tracer_writes_one_line_per_node() {
        let bus = EventBus::new();
        let builder = Arc::new(RunTreeBuilder::new());
        builder.clone().attach(&bus);
        let parent = RunId::from("r-1");
        let child = RunId::from("r-2");
        bus.emit_run(
            Event::AgentTurn {
                agent_id: AgentId::from("a"),
                input_tokens: 1,
                output_tokens: 1,
                finish_reason: None,
                elapsed_ms: 1,
            },
            parent.clone(),
            None,
        );
        bus.emit_run(
            Event::ToolInvoked {
                tool_id: ToolId::from("t"),
                args_hash: 1,
                elapsed_ms: 1,
                ok: true,
            },
            child,
            Some(parent),
        );
        let (tracer, sink) = JsonlTracer::in_memory(builder);
        tracer.flush().await.unwrap();
        let lines = sink.lines.lock().clone();
        assert_eq!(lines.len(), 2);
    }

    #[tokio::test]
    async fn langsmith_tracer_emits_records_with_project() {
        let bus = EventBus::new();
        let builder = Arc::new(RunTreeBuilder::new());
        builder.clone().attach(&bus);
        let id = RunId::from("only");
        bus.emit_run(
            Event::AgentTurn {
                agent_id: AgentId::from("a"),
                input_tokens: 1,
                output_tokens: 1,
                finish_reason: None,
                elapsed_ms: 1,
            },
            id,
            None,
        );
        let (tracer, sink) = LangSmithTracer::in_memory(builder, "atomr-test");
        tracer.flush().await.unwrap();
        let lines = sink.lines.lock().clone();
        assert_eq!(lines.len(), 1);
        let parsed: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(parsed["project"], "atomr-test");
        assert_eq!(parsed["run_type"], "chain");
    }
}
