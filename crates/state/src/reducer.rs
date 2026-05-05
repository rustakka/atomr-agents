//! Reducers — per-channel merge functions.

use std::sync::Arc;

use atomr_agents_core::Value;

/// Type-erased reducer used by the channel registry. Implementations
/// must be pure (no I/O), commutative? — no, *associative* — so
/// parallel writes within a single super-step can be folded in any
/// order without losing data.
pub type DynReducer = Arc<dyn Fn(Value, Value) -> Value + Send + Sync>;

/// Convenience trait for typed reducers; implementers are wrapped in
/// `reducer_box` to produce a `DynReducer`.
pub trait Reducer: Send + Sync + 'static {
    fn reduce(&self, current: Value, incoming: Value) -> Value;
}

/// Wrap any `Reducer` in a type-erased `DynReducer`.
pub fn reducer_box<R: Reducer>(r: R) -> DynReducer {
    Arc::new(move |current, incoming| r.reduce(current, incoming))
}

// --------------------------------------------------------------------
// LastWriteWins
// --------------------------------------------------------------------

pub struct LastWriteWins;

impl Reducer for LastWriteWins {
    fn reduce(&self, _current: Value, incoming: Value) -> Value {
        incoming
    }
}

// --------------------------------------------------------------------
// AppendList — operator.add for lists
// --------------------------------------------------------------------

pub struct AppendList;

impl Reducer for AppendList {
    fn reduce(&self, current: Value, incoming: Value) -> Value {
        let mut out = match current {
            Value::Array(v) => v,
            Value::Null => Vec::new(),
            other => vec![other],
        };
        match incoming {
            Value::Array(v) => out.extend(v),
            Value::Null => {}
            other => out.push(other),
        }
        Value::Array(out)
    }
}

// --------------------------------------------------------------------
// AppendMessages — append-with-id-dedup
// --------------------------------------------------------------------

/// Mirrors LangGraph's `add_messages`. Each message must be an object
/// with a string `id` field; messages with the same id replace the
/// existing entry instead of duplicating.
pub struct AppendMessages;

impl Reducer for AppendMessages {
    fn reduce(&self, current: Value, incoming: Value) -> Value {
        let mut out: Vec<Value> = match current {
            Value::Array(v) => v,
            Value::Null => Vec::new(),
            other => vec![other],
        };
        let new_msgs: Vec<Value> = match incoming {
            Value::Array(v) => v,
            Value::Null => Vec::new(),
            other => vec![other],
        };
        for m in new_msgs {
            let new_id = m.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
            match new_id {
                Some(id) => {
                    if let Some(slot) = out.iter_mut().find(|x| {
                        x.get("id").and_then(|v| v.as_str()) == Some(id.as_str())
                    }) {
                        *slot = m;
                    } else {
                        out.push(m);
                    }
                }
                None => out.push(m),
            }
        }
        Value::Array(out)
    }
}

// --------------------------------------------------------------------
// MergeMap — shallow object merge (incoming overrides on key clash)
// --------------------------------------------------------------------

pub struct MergeMap;

impl Reducer for MergeMap {
    fn reduce(&self, current: Value, incoming: Value) -> Value {
        let mut base = match current {
            Value::Object(m) => m,
            _ => serde_json::Map::new(),
        };
        if let Value::Object(m) = incoming {
            for (k, v) in m {
                base.insert(k, v);
            }
        }
        Value::Object(base)
    }
}

// --------------------------------------------------------------------
// MaxByTimestamp — pick the value with the higher `ts_ms` field
// --------------------------------------------------------------------

pub struct MaxByTimestamp;

impl Reducer for MaxByTimestamp {
    fn reduce(&self, current: Value, incoming: Value) -> Value {
        let cur_ts = current.get("ts_ms").and_then(|v| v.as_i64()).unwrap_or(i64::MIN);
        let new_ts = incoming.get("ts_ms").and_then(|v| v.as_i64()).unwrap_or(i64::MIN);
        if new_ts >= cur_ts {
            incoming
        } else {
            current
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn append_list_concatenates() {
        let r = AppendList;
        let a = r.reduce(json!([1, 2]), json!([3]));
        assert_eq!(a, json!([1, 2, 3]));
        let b = r.reduce(Value::Null, json!([1]));
        assert_eq!(b, json!([1]));
    }

    #[test]
    fn append_messages_dedups_by_id() {
        let r = AppendMessages;
        let a = r.reduce(
            json!([{"id": "m1", "role": "user", "text": "old"}]),
            json!([{"id": "m1", "role": "user", "text": "new"}, {"id": "m2", "role": "assistant", "text": "ok"}]),
        );
        assert_eq!(a[0]["text"], "new");
        assert_eq!(a[1]["id"], "m2");
        assert_eq!(a.as_array().unwrap().len(), 2);
    }

    #[test]
    fn merge_map_shallow() {
        let r = MergeMap;
        let a = r.reduce(json!({"a": 1, "b": 2}), json!({"b": 3, "c": 4}));
        assert_eq!(a, json!({"a": 1, "b": 3, "c": 4}));
    }

    #[test]
    fn max_by_timestamp_picks_newer() {
        let r = MaxByTimestamp;
        let a = r.reduce(
            json!({"ts_ms": 100, "v": "old"}),
            json!({"ts_ms": 200, "v": "new"}),
        );
        assert_eq!(a["v"], "new");
        let b = r.reduce(
            json!({"ts_ms": 200, "v": "kept"}),
            json!({"ts_ms": 100, "v": "rejected"}),
        );
        assert_eq!(b["v"], "kept");
    }
}
