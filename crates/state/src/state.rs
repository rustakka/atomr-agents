//! `RunState` — the runtime container per workflow run.

use std::collections::HashMap;
use std::sync::Arc;

use atomr_agents_core::{AgentError, Result, Value};

use crate::schema::StateSchema;

#[derive(Clone, Debug)]
pub struct RunState {
    schema: Arc<StateSchema>,
    values: HashMap<String, Value>,
    super_step: u64,
}

impl RunState {
    pub fn new(schema: Arc<StateSchema>) -> Self {
        let values = schema.defaults();
        Self {
            schema,
            values,
            super_step: 0,
        }
    }

    pub fn from_snapshot(schema: Arc<StateSchema>, values: HashMap<String, Value>, super_step: u64) -> Self {
        Self {
            schema,
            values,
            super_step,
        }
    }

    pub fn super_step(&self) -> u64 {
        self.super_step
    }

    pub fn advance(&mut self) {
        self.super_step += 1;
    }

    pub fn read(&self, key: &str) -> &Value {
        self.values.get(key).unwrap_or(&Value::Null)
    }

    pub fn snapshot(&self) -> HashMap<String, Value> {
        self.values.clone()
    }

    /// Apply a single write through the channel's reducer.
    pub fn write(&mut self, key: &str, value: Value) -> Result<()> {
        let channel = self
            .schema
            .channel(key)
            .ok_or_else(|| AgentError::Internal(format!("unknown channel '{key}'")))?;
        let current = self.values.remove(key).unwrap_or(Value::Null);
        let merged = (channel.reducer)(current, value);
        self.values.insert(key.to_string(), merged);
        Ok(())
    }

    /// Apply a batch of writes (typically all writes from one
    /// super-step, including any from parallel branches). Writes to
    /// unknown keys error; reducers handle ordering associatively.
    pub fn merge_writes(&mut self, writes: Vec<(String, Value)>) -> Result<()> {
        for (k, v) in writes {
            self.write(&k, v)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reducer::{AppendMessages, MergeMap};
    use crate::schema::StateSchema;
    use serde_json::json;

    fn schema() -> Arc<StateSchema> {
        Arc::new(
            StateSchema::builder()
                .add("messages", AppendMessages)
                .add("config", MergeMap)
                .build(),
        )
    }

    #[test]
    fn writes_route_to_correct_reducers() {
        let mut s = RunState::new(schema());
        s.write("messages", json!([{"id": "m1", "role": "user", "text": "hi"}]))
            .unwrap();
        s.write("messages", json!([{"id": "m1", "role": "user", "text": "edit"}]))
            .unwrap();
        s.write("config", json!({"a": 1})).unwrap();
        s.write("config", json!({"b": 2})).unwrap();
        assert_eq!(s.read("messages")[0]["text"], "edit");
        assert_eq!(s.read("config"), &json!({"a": 1, "b": 2}));
    }

    #[test]
    fn unknown_channel_errors() {
        let mut s = RunState::new(schema());
        assert!(s.write("nonexistent", Value::Null).is_err());
    }

    #[test]
    fn merge_writes_applies_in_order() {
        let mut s = RunState::new(schema());
        s.merge_writes(vec![
            (
                "messages".into(),
                json!([{"id": "m1", "role": "user", "text": "a"}]),
            ),
            (
                "messages".into(),
                json!([{"id": "m2", "role": "assistant", "text": "b"}]),
            ),
        ])
        .unwrap();
        assert_eq!(s.read("messages").as_array().unwrap().len(), 2);
    }
}
