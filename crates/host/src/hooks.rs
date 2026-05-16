//! M5 — Hook registry + dispatcher.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{HostError, HostResult};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum HookWhen {
    #[serde(rename = "pre")]
    Pre,
    #[default]
    #[serde(rename = "post")]
    Post,
    #[serde(rename = "both")]
    Both,
}

impl HookWhen {
    pub fn matches(&self, requested: &HookWhen) -> bool {
        matches!(
            (self, requested),
            (HookWhen::Both, _)
                | (HookWhen::Pre, HookWhen::Pre)
                | (HookWhen::Post, HookWhen::Post)
        )
    }
}

pub type BuiltinFn =
    Arc<dyn Fn(&serde_json::Value) -> HostResult<serde_json::Value> + Send + Sync + 'static>;

#[derive(Clone)]
pub struct HookSpec {
    pub id: String,
    pub event: String,
    pub when: HookWhen,
    pub match_: HashMap<String, serde_json::Value>,
    pub call: HookCall,
    pub timeout_ms: u64,
}

#[derive(Clone)]
pub enum HookCall {
    Builtin(String, BuiltinFn),
    External(serde_json::Value),
}

#[derive(Debug, Clone, Serialize)]
pub struct HookResult {
    pub hook_id: String,
    pub event: String,
    pub when: HookWhen,
    pub ok: bool,
    pub output: Option<serde_json::Value>,
    pub error: Option<String>,
    pub duration_ms: u64,
}

#[derive(Default, Clone)]
pub struct HookRegistry {
    inner: Arc<Mutex<HookRegistryInner>>,
}

#[derive(Default)]
struct HookRegistryInner {
    hooks: Vec<HookSpec>,
    builtins: HashMap<String, BuiltinFn>,
}

impl HookRegistry {
    pub fn new() -> Self {
        let me = Self::default();
        me.register_builtin("redact_secrets", builtin_redact_secrets);
        me.register_builtin("record_to_jsonl", builtin_record_to_jsonl);
        me
    }

    pub fn register_builtin<F>(&self, id: &str, f: F)
    where
        F: Fn(&serde_json::Value) -> HostResult<serde_json::Value> + Send + Sync + 'static,
    {
        let mut inner = self.inner.lock();
        inner.builtins.insert(id.to_string(), Arc::new(f));
    }

    pub fn register(&self, spec: HookSpec) {
        self.inner.lock().hooks.push(spec);
    }

    pub fn list(&self) -> Vec<HookSpec> {
        self.inner.lock().hooks.clone()
    }

    pub fn builtin(&self, id: &str) -> Option<BuiltinFn> {
        self.inner.lock().builtins.get(id).cloned()
    }
}

pub fn match_payload(spec_match: &HashMap<String, serde_json::Value>, payload: &serde_json::Value) -> bool {
    if spec_match.is_empty() {
        return true;
    }
    for (k, v) in spec_match {
        let actual = payload.get(k);
        if actual != Some(v) {
            return false;
        }
    }
    true
}

pub struct HookDispatcher {
    registry: HookRegistry,
}

impl HookDispatcher {
    pub fn new(registry: HookRegistry) -> Self {
        Self { registry }
    }

    pub async fn dispatch(
        &self,
        event: &str,
        when: HookWhen,
        payload: serde_json::Value,
    ) -> Vec<HookResult> {
        let hooks: Vec<HookSpec> = self
            .registry
            .list()
            .into_iter()
            .filter(|h| h.event == event && h.when.matches(&when) && match_payload(&h.match_, &payload))
            .collect();
        let mut handles = Vec::with_capacity(hooks.len());
        for spec in hooks {
            let payload = payload.clone();
            let when = when.clone();
            let timeout = Duration::from_millis(spec.timeout_ms.max(1));
            let handle = tokio::spawn(async move {
                let start = Instant::now();
                let res = tokio::time::timeout(timeout, async {
                    match &spec.call {
                        HookCall::Builtin(_, f) => f(&payload),
                        HookCall::External(_) => Err(HostError::HookDispatch(
                            "external hook callables are wired in M9".into(),
                        )),
                    }
                })
                .await;
                let elapsed = start.elapsed().as_millis() as u64;
                match res {
                    Ok(Ok(out)) => HookResult {
                        hook_id: spec.id.clone(),
                        event: spec.event.clone(),
                        when,
                        ok: true,
                        output: Some(out),
                        error: None,
                        duration_ms: elapsed,
                    },
                    Ok(Err(e)) => HookResult {
                        hook_id: spec.id.clone(),
                        event: spec.event.clone(),
                        when,
                        ok: false,
                        output: None,
                        error: Some(e.to_string()),
                        duration_ms: elapsed,
                    },
                    Err(_) => HookResult {
                        hook_id: spec.id.clone(),
                        event: spec.event.clone(),
                        when,
                        ok: false,
                        output: None,
                        error: Some(format!("timed out after {}ms", spec.timeout_ms)),
                        duration_ms: elapsed,
                    },
                }
            });
            handles.push(handle);
        }
        let mut out = Vec::with_capacity(handles.len());
        for h in handles {
            if let Ok(r) = h.await {
                out.push(r);
            }
        }
        out
    }
}

// ---------- built-in hook implementations -----------------------------------

const SECRET_PATTERNS: &[&str] = &[
    r"sk-[A-Za-z0-9_\-]{16,}",
    r"AKIA[0-9A-Z]{16}",
    r"ghp_[A-Za-z0-9]{20,}",
    r"(?i)api[_-]?key\s*=\s*[A-Za-z0-9_\-]{8,}",
];

pub fn builtin_redact_secrets(
    payload: &serde_json::Value,
) -> HostResult<serde_json::Value> {
    let text = payload
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let mut out = text.to_string();
    for pat in SECRET_PATTERNS {
        if let Ok(re) = Regex::new(pat) {
            out = re.replace_all(&out, "[REDACTED]").to_string();
        }
    }
    Ok(serde_json::json!({ "text": out }))
}

pub fn builtin_record_to_jsonl(
    payload: &serde_json::Value,
) -> HostResult<serde_json::Value> {
    let target = payload
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| HostError::HookDispatch("record_to_jsonl: missing `path`".into()))?;
    let entry = payload.get("entry").cloned().unwrap_or(serde_json::json!({}));
    let line = serde_json::to_string(&entry)
        .map_err(|e| HostError::HookDispatch(format!("record_to_jsonl: serialize: {e}")))?;
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(target)
        .map_err(|e| HostError::HookDispatch(format!("record_to_jsonl: open {target}: {e}")))?;
    writeln!(f, "{line}").map_err(|e| HostError::HookDispatch(format!("record_to_jsonl: write: {e}")))?;
    Ok(serde_json::json!({ "wrote": target }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dispatcher_runs_matching_hooks() {
        let reg = HookRegistry::new();
        let f = reg.builtin("redact_secrets").unwrap();
        reg.register(HookSpec {
            id: "redact".into(),
            event: "on_tool_call".into(),
            when: HookWhen::Post,
            match_: Default::default(),
            call: HookCall::Builtin("redact_secrets".into(), f),
            timeout_ms: 1_000,
        });
        let d = HookDispatcher::new(reg);
        let res = d
            .dispatch(
                "on_tool_call",
                HookWhen::Post,
                serde_json::json!({"text": "api_key=sk-12345678abcd"}),
            )
            .await;
        assert_eq!(res.len(), 1);
        assert!(res[0].ok);
        let out_text = res[0].output.as_ref().unwrap().get("text").unwrap().as_str().unwrap();
        assert!(out_text.contains("[REDACTED]"));
    }

    #[test]
    fn match_payload_supports_partial() {
        let mut m = HashMap::new();
        m.insert("tool".into(), serde_json::json!("shell.exec"));
        assert!(match_payload(&m, &serde_json::json!({"tool":"shell.exec","text":"…"})));
        assert!(!match_payload(&m, &serde_json::json!({"tool":"other"})));
    }
}
