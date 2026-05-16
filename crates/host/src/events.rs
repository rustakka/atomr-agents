//! M9 — Append-only JSONL event log.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::error::{HostError, HostResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRecord {
    pub ts_ms: u64,
    pub kind: String,
    #[serde(default)]
    pub agent_id: Option<String>,
    pub payload: serde_json::Value,
}

#[derive(Clone)]
pub struct EventLog {
    path: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl EventLog {
    pub fn new(path: PathBuf) -> Self {
        Self { path, lock: Arc::new(Mutex::new(())) }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn append(&self, rec: &EventRecord) -> HostResult<()> {
        let _g = self.lock.lock();
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| HostError::io(parent, e))?;
        }
        let line = serde_json::to_string(rec).map_err(|e| HostError::json(self.path.clone(), e))?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| HostError::io(&self.path, e))?;
        writeln!(f, "{line}").map_err(|e| HostError::io(&self.path, e))?;
        Ok(())
    }

    pub fn emit(&self, kind: impl Into<String>, agent_id: Option<String>, payload: serde_json::Value) -> HostResult<()> {
        self.append(&EventRecord {
            ts_ms: now_ms(),
            kind: kind.into(),
            agent_id,
            payload,
        })
    }

    pub fn read_all(&self) -> HostResult<Vec<EventRecord>> {
        if !self.path.is_file() {
            return Ok(Vec::new());
        }
        let f = std::fs::File::open(&self.path).map_err(|e| HostError::io(&self.path, e))?;
        let mut out = Vec::new();
        for line in BufReader::new(f).lines() {
            let line = line.map_err(|e| HostError::io(&self.path, e))?;
            if line.trim().is_empty() {
                continue;
            }
            let rec: EventRecord = serde_json::from_str(&line)
                .map_err(|e| HostError::json(self.path.clone(), e))?;
            out.push(rec);
        }
        Ok(out)
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn append_and_read_roundtrip() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("events.jsonl");
        let log = EventLog::new(path);
        log.emit("first", Some("alpha".into()), serde_json::json!({"x":1})).unwrap();
        log.emit("second", None, serde_json::json!({"y":2})).unwrap();
        let all = log.read_all().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].kind, "first");
        assert_eq!(all[1].kind, "second");
    }
}
