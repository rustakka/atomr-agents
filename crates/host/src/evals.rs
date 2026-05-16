//! M12 — Eval harness.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{HostError, HostResult};
use crate::layout::HostPaths;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalCase {
    pub id: String,
    pub input: String,
    pub expected: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalSuite {
    pub id: String,
    pub scorer: String,
    #[serde(default)]
    pub description: Option<String>,
    pub cases: Vec<EvalCase>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalCaseResult {
    pub case_id: String,
    pub passed: bool,
    pub score: f64,
    pub reason: Option<String>,
    pub output: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalRun {
    pub suite_id: String,
    pub agent_id: String,
    pub results: Vec<EvalCaseResult>,
    pub passed: usize,
    pub total: usize,
}

impl EvalRun {
    pub fn pass_rate(&self) -> f64 {
        if self.total == 0 {
            return 1.0;
        }
        self.passed as f64 / self.total as f64
    }
}

pub fn load_suite(host: &HostPaths, suite_id: &str) -> HostResult<EvalSuite> {
    let dir = host.evals_dir();
    for ext in ["yaml", "yml", "json"] {
        let p = dir.join(format!("{suite_id}.{ext}"));
        if p.is_file() {
            return load_suite_at(&p);
        }
    }
    Err(HostError::Eval(format!(
        "no suite `{suite_id}` under {}",
        dir.display()
    )))
}

pub fn load_suite_at(path: &Path) -> HostResult<EvalSuite> {
    let text = std::fs::read_to_string(path).map_err(|e| HostError::io(path, e))?;
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("yaml");
    let suite: EvalSuite = if ext == "json" {
        serde_json::from_str(&text).map_err(|e| HostError::json(path.to_path_buf(), e))?
    } else {
        serde_yaml::from_str(&text).map_err(|e| HostError::yaml(path.to_path_buf(), e))?
    };
    Ok(suite)
}

pub fn list_suites(host: &HostPaths) -> HostResult<Vec<String>> {
    let dir = host.evals_dir();
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|e| HostError::io(&dir, e))? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let p = entry.path();
        let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
        if !matches!(ext, "yaml" | "yml" | "json") {
            continue;
        }
        if let Some(stem) = p.file_stem().and_then(|n| n.to_str()) {
            out.push(stem.to_string());
        }
    }
    out.sort();
    Ok(out)
}

pub fn scaffold_suite(host: &HostPaths, suite_id: &str) -> HostResult<PathBuf> {
    let dir = host.evals_dir();
    std::fs::create_dir_all(&dir).map_err(|e| HostError::io(&dir, e))?;
    let path = dir.join(format!("{suite_id}.yaml"));
    if path.exists() {
        return Err(HostError::Eval(format!("{} already exists", path.display())));
    }
    let body = format!(
        "id: {suite_id}\nscorer: contains\ndescription: Smoke check.\ncases:\n  - id: identity\n    input: hello\n    expected:\n      contains:\n        - default\n"
    );
    std::fs::write(&path, body).map_err(|e| HostError::io(&path, e))?;
    Ok(path)
}

pub type Scorer = fn(&serde_json::Value, &str) -> EvalCaseResultRaw;

pub struct EvalCaseResultRaw {
    pub passed: bool,
    pub score: f64,
    pub reason: Option<String>,
}

pub fn scorer_for(name: &str) -> HostResult<Scorer> {
    match name {
        "contains" => Ok(scorer_contains),
        "excludes" => Ok(scorer_excludes),
        "regex" => Ok(scorer_regex),
        other => Err(HostError::Eval(format!("unknown scorer `{other}`"))),
    }
}

fn extract_substrings(expected: &serde_json::Value, key: &str) -> Vec<String> {
    expected
        .get(key)
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default()
}

pub fn scorer_contains(expected: &serde_json::Value, output: &str) -> EvalCaseResultRaw {
    let subs = extract_substrings(expected, "contains");
    if subs.is_empty() {
        return EvalCaseResultRaw {
            passed: false,
            score: 0.0,
            reason: Some("`contains` expected to be a non-empty list".into()),
        };
    }
    let missing: Vec<&String> = subs.iter().filter(|s| !output.contains(s.as_str())).collect();
    let total = subs.len();
    let matched = total - missing.len();
    let score = matched as f64 / total as f64;
    if missing.is_empty() {
        EvalCaseResultRaw { passed: true, score: 1.0, reason: None }
    } else {
        EvalCaseResultRaw {
            passed: false,
            score,
            reason: Some(format!(
                "missing {}/{} substring(s): {:?}",
                missing.len(),
                total,
                missing
            )),
        }
    }
}

pub fn scorer_excludes(expected: &serde_json::Value, output: &str) -> EvalCaseResultRaw {
    let subs = extract_substrings(expected, "excludes");
    let hits: Vec<&String> = subs.iter().filter(|s| output.contains(s.as_str())).collect();
    if hits.is_empty() {
        EvalCaseResultRaw { passed: true, score: 1.0, reason: None }
    } else {
        EvalCaseResultRaw {
            passed: false,
            score: 0.0,
            reason: Some(format!("forbidden substring(s) appeared: {:?}", hits)),
        }
    }
}

pub fn scorer_regex(expected: &serde_json::Value, output: &str) -> EvalCaseResultRaw {
    let pat = match expected.get("regex").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => {
            return EvalCaseResultRaw {
                passed: false,
                score: 0.0,
                reason: Some("`regex` expected to be a string".into()),
            };
        }
    };
    let re = match Regex::new(pat) {
        Ok(r) => r,
        Err(e) => {
            return EvalCaseResultRaw {
                passed: false,
                score: 0.0,
                reason: Some(format!("invalid regex `{pat}`: {e}")),
            };
        }
    };
    if re.is_search_match(output) {
        EvalCaseResultRaw { passed: true, score: 1.0, reason: None }
    } else {
        EvalCaseResultRaw {
            passed: false,
            score: 0.0,
            reason: Some(format!("pattern `{pat}` did not match")),
        }
    }
}

// regex::Regex doesn't have is_search_match — wrapper for clarity.
trait RegexSearchExt {
    fn is_search_match(&self, hay: &str) -> bool;
}
impl RegexSearchExt for Regex {
    fn is_search_match(&self, hay: &str) -> bool {
        self.is_match(hay)
    }
}

/// Run a suite synchronously against an arbitrary responder function.
pub fn run_suite_sync<F>(
    suite: &EvalSuite,
    agent_id: &str,
    responder: F,
) -> HostResult<EvalRun>
where
    F: Fn(&str) -> String,
{
    let scorer = scorer_for(&suite.scorer)?;
    let mut results = Vec::with_capacity(suite.cases.len());
    let mut passed = 0usize;
    for case in &suite.cases {
        let output = responder(&case.input);
        let raw = scorer(&case.expected, &output);
        if raw.passed {
            passed += 1;
        }
        results.push(EvalCaseResult {
            case_id: case.id.clone(),
            passed: raw.passed,
            score: raw.score,
            reason: raw.reason,
            output,
        });
    }
    Ok(EvalRun {
        suite_id: suite.id.clone(),
        agent_id: agent_id.to_string(),
        total: suite.cases.len(),
        passed,
        results,
    })
}

// keep imports clean - HashMap unused-import shim.
#[allow(dead_code)]
fn _hashmap_import_keep() -> HashMap<(), ()> {
    HashMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn contains_scorer_partial() {
        let r = scorer_contains(&serde_json::json!({"contains":["a","b"]}), "only a here");
        assert!(!r.passed);
        assert!(r.reason.unwrap().contains("missing 1/2"));
    }

    #[test]
    fn run_suite_against_responder() {
        let suite = EvalSuite {
            id: "smoke".into(),
            scorer: "contains".into(),
            description: None,
            cases: vec![EvalCase {
                id: "c1".into(),
                input: "hello".into(),
                expected: serde_json::json!({"contains":["hello"]}),
            }],
        };
        let run = run_suite_sync(&suite, "alpha", |u| format!("echo: {u}")).unwrap();
        assert_eq!(run.passed, 1);
        assert_eq!(run.total, 1);
    }

    #[test]
    fn scaffold_and_list() {
        let tmp = tempdir().unwrap();
        let host = HostPaths::new(tmp.path());
        scaffold_suite(&host, "smoke").unwrap();
        assert_eq!(list_suites(&host).unwrap(), vec!["smoke".to_string()]);
    }
}
