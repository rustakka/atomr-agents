//! Memory tools — Tool implementations that an agent can call to
//! write / update / recall long-term memories. These mirror the
//! "LangMem" pattern.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentError, InvokeCtx, Result, ToolId, Value};
use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};

use crate::long_term::{LongStore, Namespace};

fn ns_from_value(v: &Value) -> Result<Namespace> {
    let arr = v
        .as_array()
        .ok_or_else(|| AgentError::Tool("namespace must be an array of strings".into()))?;
    Ok(Namespace(
        arr.iter().map(|x| x.as_str().unwrap_or("").to_string()).collect(),
    ))
}

// --------------------------------------------------------------------
// WriteMemoryTool
// --------------------------------------------------------------------

pub struct WriteMemoryTool {
    pub store: Arc<dyn LongStore>,
    descriptor: ToolDescriptor,
}

impl WriteMemoryTool {
    pub fn new(store: Arc<dyn LongStore>) -> Self {
        Self {
            store,
            descriptor: ToolDescriptor {
                id: ToolId::from("write_memory"),
                name: "write_memory".into(),
                description: "Persist a key/value pair to the agent's long-term memory under a namespace."
                    .into(),
                schema: ToolSchema(serde_json::json!({
                    "type": "object",
                    "required": ["namespace", "key", "value"],
                    "properties": {
                        "namespace": {"type": "array", "items": {"type": "string"}},
                        "key": {"type": "string"},
                        "value": {},
                    }
                })),
            },
        }
    }
}

#[async_trait]
impl Tool for WriteMemoryTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }
    async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> Result<Value> {
        let ns = ns_from_value(args.get("namespace").unwrap_or(&Value::Null))?;
        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("missing 'key'".into()))?
            .to_string();
        let value = args.get("value").cloned().unwrap_or(Value::Null);
        self.store.put(&ns, &key, value, None).await?;
        Ok(serde_json::json!({"ok": true, "key": key}))
    }
}

// --------------------------------------------------------------------
// UpdateMemoryTool — alias of WriteMemory but signals intent to
// replace and returns the previous value, if any.
// --------------------------------------------------------------------

pub struct UpdateMemoryTool {
    pub store: Arc<dyn LongStore>,
    descriptor: ToolDescriptor,
}

impl UpdateMemoryTool {
    pub fn new(store: Arc<dyn LongStore>) -> Self {
        Self {
            store,
            descriptor: ToolDescriptor {
                id: ToolId::from("update_memory"),
                name: "update_memory".into(),
                description: "Replace a long-term memory value; returns the previous value.".into(),
                schema: ToolSchema(serde_json::json!({
                    "type": "object",
                    "required": ["namespace", "key", "value"],
                    "properties": {
                        "namespace": {"type": "array", "items": {"type": "string"}},
                        "key": {"type": "string"},
                        "value": {},
                    }
                })),
            },
        }
    }
}

#[async_trait]
impl Tool for UpdateMemoryTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }
    async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> Result<Value> {
        let ns = ns_from_value(args.get("namespace").unwrap_or(&Value::Null))?;
        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("missing 'key'".into()))?
            .to_string();
        let new = args.get("value").cloned().unwrap_or(Value::Null);
        let prev = self.store.get(&ns, &key).await?.map(|i| i.value);
        self.store.put(&ns, &key, new, None).await?;
        Ok(serde_json::json!({"ok": true, "previous": prev}))
    }
}

// --------------------------------------------------------------------
// RecallMemoryTool
// --------------------------------------------------------------------

pub struct RecallMemoryTool {
    pub store: Arc<dyn LongStore>,
    descriptor: ToolDescriptor,
}

impl RecallMemoryTool {
    pub fn new(store: Arc<dyn LongStore>) -> Self {
        Self {
            store,
            descriptor: ToolDescriptor {
                id: ToolId::from("recall_memory"),
                name: "recall_memory".into(),
                description:
                    "Search the agent's long-term memory under a namespace; returns the top-k most relevant items."
                        .into(),
                schema: ToolSchema(serde_json::json!({
                    "type": "object",
                    "required": ["namespace"],
                    "properties": {
                        "namespace": {"type": "array", "items": {"type": "string"}},
                        "top_k": {"type": "integer", "default": 5},
                    }
                })),
            },
        }
    }
}

#[async_trait]
impl Tool for RecallMemoryTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }
    async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> Result<Value> {
        let ns = ns_from_value(args.get("namespace").unwrap_or(&Value::Null))?;
        let top_k = args.get("top_k").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
        let hits = self.store.search(&ns, None, top_k).await?;
        let items: Vec<Value> = hits
            .into_iter()
            .map(|i| serde_json::json!({"key": i.key, "value": i.value}))
            .collect();
        Ok(serde_json::json!({"items": items}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::long_term::InMemoryLongStore;
    use atomr_agents_core::{CallCtx, IterationBudget, MoneyBudget, TimeBudget, TokenBudget};
    use std::time::Duration;

    fn ictx() -> InvokeCtx {
        InvokeCtx {
            call: CallCtx {
                agent_id: None,
                tokens: TokenBudget::new(1000),
                time: TimeBudget::new(Duration::from_secs(5)),
                money: MoneyBudget::from_usd(0.10),
                iterations: IterationBudget::new(5),
                trace: vec![],
            },
            tool_call_id: "t1".into(),
            raw_args: Value::Null,
        }
    }

    #[tokio::test]
    async fn write_then_recall_round_trips() {
        let store: Arc<dyn LongStore> = Arc::new(InMemoryLongStore::new());
        let writer = WriteMemoryTool::new(store.clone());
        writer
            .invoke(
                serde_json::json!({
                    "namespace": ["user", "alice", "facts"],
                    "key": "city",
                    "value": "Boston",
                }),
                &ictx(),
            )
            .await
            .unwrap();
        let recaller = RecallMemoryTool::new(store);
        let v = recaller
            .invoke(
                serde_json::json!({"namespace": ["user", "alice", "facts"], "top_k": 5}),
                &ictx(),
            )
            .await
            .unwrap();
        assert_eq!(v["items"].as_array().unwrap().len(), 1);
        assert_eq!(v["items"][0]["value"], "Boston");
    }

    #[tokio::test]
    async fn update_returns_previous() {
        let store: Arc<dyn LongStore> = Arc::new(InMemoryLongStore::new());
        let updater = UpdateMemoryTool::new(store.clone());
        let _ = updater
            .invoke(
                serde_json::json!({
                    "namespace": ["k"],
                    "key": "x",
                    "value": 1,
                }),
                &ictx(),
            )
            .await
            .unwrap();
        let v = updater
            .invoke(
                serde_json::json!({
                    "namespace": ["k"],
                    "key": "x",
                    "value": 2,
                }),
                &ictx(),
            )
            .await
            .unwrap();
        assert_eq!(v["previous"], 1);
    }
}
