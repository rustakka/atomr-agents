//! State-schema declaration. A schema is a registry of channels,
//! each with its own reducer and default value.

use std::collections::HashMap;
use std::sync::Arc;

use atomr_agents_core::Value;

use crate::reducer::{reducer_box, DynReducer, LastWriteWins, Reducer};

#[derive(Clone)]
pub struct Channel {
    pub key: String,
    pub reducer: DynReducer,
    pub default: Value,
}

impl std::fmt::Debug for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Channel")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub struct StateSchema {
    channels: HashMap<String, Channel>,
}

impl StateSchema {
    pub fn builder() -> StateSchemaBuilder {
        StateSchemaBuilder {
            channels: HashMap::new(),
        }
    }

    pub fn channel(&self, key: &str) -> Option<&Channel> {
        self.channels.get(key)
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.channels.keys().map(|s| s.as_str())
    }

    pub fn defaults(&self) -> HashMap<String, Value> {
        self.channels
            .iter()
            .map(|(k, c)| (k.clone(), c.default.clone()))
            .collect()
    }
}

pub struct StateSchemaBuilder {
    channels: HashMap<String, Channel>,
}

impl StateSchemaBuilder {
    /// Add a channel with a typed reducer.
    pub fn add<R: Reducer>(mut self, key: impl Into<String>, reducer: R) -> Self {
        let key = key.into();
        self.channels.insert(
            key.clone(),
            Channel {
                key,
                reducer: reducer_box(reducer),
                default: Value::Null,
            },
        );
        self
    }

    /// Add a channel with a typed reducer and an explicit default.
    pub fn add_with_default<R: Reducer>(
        mut self,
        key: impl Into<String>,
        reducer: R,
        default: Value,
    ) -> Self {
        let key = key.into();
        self.channels.insert(
            key.clone(),
            Channel {
                key,
                reducer: reducer_box(reducer),
                default,
            },
        );
        self
    }

    /// Default any unspecified key to `LastWriteWins` for convenience.
    pub fn add_lww(mut self, key: impl Into<String>) -> Self {
        let key = key.into();
        self.channels.insert(
            key.clone(),
            Channel {
                key,
                reducer: Arc::new(|_, n| n) as DynReducer,
                default: Value::Null,
            },
        );
        self
    }

    pub fn build(self) -> StateSchema {
        StateSchema {
            channels: self.channels,
        }
    }
}

#[allow(dead_code)]
fn _last_write_wins_in_scope(_l: LastWriteWins) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reducer::{AppendMessages, MergeMap};
    use serde_json::json;

    #[test]
    fn schema_lookup_returns_reducer() {
        let s = StateSchema::builder()
            .add("messages", AppendMessages)
            .add_with_default("config", MergeMap, json!({"v": 0}))
            .build();
        assert!(s.channel("messages").is_some());
        assert_eq!(s.channel("config").unwrap().default, json!({"v": 0}));
    }
}
