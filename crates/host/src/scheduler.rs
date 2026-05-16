//! M6 — `Scheduler` for cron-like recurring fires.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::error::{HostError, HostResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronEntry {
    pub id: String,
    pub expression: String,
    pub call: serde_json::Value,
    #[serde(default)]
    pub input: serde_json::Value,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone)]
pub struct CronFireResult {
    pub cron_id: String,
    pub fired_at_ms: u64,
    pub call: serde_json::Value,
    pub input: serde_json::Value,
}

/// Parse an `every:Nu` expression where `u ∈ {s,m,h,d}`. Returns the
/// interval in seconds.
pub fn parse_expression(expr: &str) -> HostResult<u64> {
    let rest = expr.strip_prefix("every:").ok_or_else(|| {
        HostError::Scheduler(format!(
            "unsupported expression `{expr}` (expected `every:Ns`/`m`/`h`/`d`)"
        ))
    })?;
    let (num, unit) = rest.split_at(rest.len() - 1);
    let n: u64 = num
        .parse()
        .map_err(|_| HostError::Scheduler(format!("invalid number in `{expr}`")))?;
    let secs = match unit {
        "s" => n,
        "m" => n * 60,
        "h" => n * 60 * 60,
        "d" => n * 60 * 60 * 24,
        other => {
            return Err(HostError::Scheduler(format!(
                "unsupported unit `{other}` in `{expr}` (use s/m/h/d)"
            )));
        }
    };
    if secs == 0 {
        return Err(HostError::Scheduler(format!("interval must be > 0 in `{expr}`")));
    }
    Ok(secs)
}

#[derive(Debug, Clone)]
pub struct Scheduler {
    inner: Arc<Mutex<SchedulerInner>>,
}

#[derive(Default, Debug)]
struct SchedulerInner {
    entries: HashMap<String, CronEntry>,
    next_due_ms: HashMap<String, u64>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl Scheduler {
    pub fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(SchedulerInner::default())) }
    }

    pub fn register(&self, entry: CronEntry) -> HostResult<u64> {
        let secs = parse_expression(&entry.expression)?;
        let next = now_ms() + secs * 1000;
        let id = entry.id.clone();
        {
            let mut inner = self.inner.lock();
            inner.entries.insert(id.clone(), entry);
            inner.next_due_ms.insert(id, next);
        }
        Ok(next)
    }

    pub fn list(&self) -> Vec<CronEntry> {
        self.inner.lock().entries.values().cloned().collect()
    }

    pub fn remove(&self, id: &str) -> bool {
        let mut inner = self.inner.lock();
        inner.next_due_ms.remove(id);
        inner.entries.remove(id).is_some()
    }

    /// Fire all entries whose `next_due_ms <= now_ms`. Reschedules each
    /// fired entry. Returns the entries that fired.
    pub fn fire_due(&self) -> Vec<CronFireResult> {
        let now = now_ms();
        let mut fired = Vec::new();
        let mut inner = self.inner.lock();
        let ids: Vec<String> = inner
            .next_due_ms
            .iter()
            .filter_map(|(id, due)| if *due <= now { Some(id.clone()) } else { None })
            .collect();
        for id in ids {
            let entry = match inner.entries.get(&id) {
                Some(e) if e.enabled => e.clone(),
                _ => continue,
            };
            fired.push(CronFireResult {
                cron_id: id.clone(),
                fired_at_ms: now,
                call: entry.call.clone(),
                input: entry.input.clone(),
            });
            let secs = parse_expression(&entry.expression).unwrap_or(60);
            inner.next_due_ms.insert(id, now + secs * 1000);
        }
        fired
    }

    /// Run a single bounded tick loop until `until_ms`, dispatching to
    /// the provided callback. Used by tests; production code uses the
    /// async [`Scheduler::run`] driver.
    pub fn tick_until(&self, until_ms: u64) -> Vec<CronFireResult> {
        let mut out = Vec::new();
        while now_ms() < until_ms {
            out.extend(self.fire_due());
            std::thread::sleep(Duration::from_millis(50));
        }
        out
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

/// Scaffold `<root>/crons/<cron-id>.yaml`.
pub fn scaffold_cron(
    crons_dir: &Path,
    cron_id: &str,
    expression: &str,
    call: serde_json::Value,
) -> HostResult<std::path::PathBuf> {
    std::fs::create_dir_all(crons_dir).map_err(|e| HostError::io(crons_dir, e))?;
    let entry = CronEntry {
        id: cron_id.to_string(),
        expression: expression.to_string(),
        call,
        input: serde_json::json!({}),
        enabled: true,
    };
    let body = serde_yaml::to_string(&entry)
        .map_err(|e| HostError::yaml(crons_dir.join(format!("{cron_id}.yaml")), e))?;
    let path = crons_dir.join(format!("{cron_id}.yaml"));
    std::fs::write(&path, body).map_err(|e| HostError::io(&path, e))?;
    Ok(path)
}

/// Load all `<root>/crons/*.yaml` into a fresh scheduler.
pub fn load_crons(crons_dir: &Path) -> HostResult<Vec<CronEntry>> {
    if !crons_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(crons_dir).map_err(|e| HostError::io(crons_dir, e))? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let p = entry.path();
        let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
        if !matches!(ext, "yaml" | "yml") {
            continue;
        }
        let text = std::fs::read_to_string(&p).map_err(|e| HostError::io(&p, e))?;
        let ce: CronEntry = serde_yaml::from_str(&text).map_err(|e| HostError::yaml(&p, e))?;
        out.push(ce);
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_expression_supports_units() {
        assert_eq!(parse_expression("every:5s").unwrap(), 5);
        assert_eq!(parse_expression("every:2m").unwrap(), 120);
        assert_eq!(parse_expression("every:1h").unwrap(), 3600);
        assert_eq!(parse_expression("every:1d").unwrap(), 86_400);
        assert!(parse_expression("every:0s").is_err());
        assert!(parse_expression("nope").is_err());
        assert!(parse_expression("every:5x").is_err());
    }

    #[test]
    fn scheduler_register_lists_removes() {
        let s = Scheduler::new();
        s.register(CronEntry {
            id: "daily".into(),
            expression: "every:1d".into(),
            call: serde_json::json!({"kind":"builtin","id":"noop"}),
            input: serde_json::json!({}),
            enabled: true,
        })
        .unwrap();
        assert_eq!(s.list().len(), 1);
        assert!(s.remove("daily"));
        assert!(s.list().is_empty());
    }
}
