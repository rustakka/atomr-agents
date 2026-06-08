//! Request / result types crossing the backend boundary.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::budget::ResourceBudget;
use crate::exit::ExitStatus;
use crate::id::{ExecId, SandboxId, SnapshotId};
use crate::profile::{Language, SandboxProfile};

/// Which backend the harness should resolve to. `Auto` lets the harness
/// pick the most secure available backend (Firecracker → Docker → Mock).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SandboxBackendSel {
    #[default]
    Auto,
    Mock,
    Docker {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        image: Option<String>,
    },
    Firecracker,
    Remote {
        endpoint: String,
    },
}

/// Provision request: cold-boot a fresh sandbox, or warm-fork from a
/// snapshot when `from_snapshot` is set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateSandbox {
    pub profile: SandboxProfile,
    /// Explicit budget override. `None` → [`ResourceBudget::for_profile`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<ResourceBudget>,
    #[serde(default)]
    pub backend: SandboxBackendSel,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Fork from a warm snapshot instead of cold-booting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_snapshot: Option<SnapshotId>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, serde_json::Value>,
}

impl CreateSandbox {
    pub fn new(profile: SandboxProfile) -> Self {
        Self {
            profile,
            budget: None,
            backend: SandboxBackendSel::default(),
            env: BTreeMap::new(),
            from_snapshot: None,
            metadata: BTreeMap::new(),
        }
    }

    /// The budget the backend should provision with. Applies the Rust floor
    /// on top of any explicit override — a caller cannot under-provision a
    /// Rust profile.
    pub fn effective_budget(&self) -> ResourceBudget {
        let base = self
            .budget
            .unwrap_or_else(|| ResourceBudget::for_profile(self.profile));
        if self.profile.is_rust() {
            base.enforce_rust_floor()
        } else {
            base
        }
    }

    pub fn with_backend(mut self, backend: SandboxBackendSel) -> Self {
        self.backend = backend;
        self
    }

    pub fn with_budget(mut self, budget: ResourceBudget) -> Self {
        self.budget = Some(budget);
        self
    }
}

/// Metadata about a live sandbox returned by the backend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SandboxInfo {
    pub id: SandboxId,
    pub profile: SandboxProfile,
    pub budget: ResourceBudget,
    /// `backend.name()` of the backend that created this sandbox.
    pub backend: String,
    /// Cold-start (or snapshot-resume) latency in milliseconds.
    pub boot_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<SnapshotId>,
    pub created_at: DateTime<Utc>,
}

/// One code-execution request inside an existing sandbox.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecRequest {
    pub language: Language,
    pub code: String,
    /// Package installs to run before the code (pip / npm / cargo names).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
}

impl ExecRequest {
    pub fn new(language: Language, code: impl Into<String>) -> Self {
        Self {
            language,
            code: code.into(),
            dependencies: Vec::new(),
            stdin: None,
            env: BTreeMap::new(),
            timeout_secs: None,
        }
    }
}

/// Captured output of a completed exec.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecResult {
    pub exec_id: ExecId,
    pub exit: ExitStatus,
    pub stdout: String,
    pub stderr: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub timed_out: bool,
}

impl ExecResult {
    /// The single canonical flat JSON view exposed to consumers — the
    /// `execute_in_sandbox` tool, the harness `Callable`, and the PyO3
    /// bindings all return this, so there is exactly one consumer-facing shape.
    pub fn to_tool_json(&self) -> serde_json::Value {
        serde_json::json!({
            "exec_id":   self.exec_id.as_str(),
            "exit_code": self.exit.code,
            "success":   self.exit.success,
            "stdout":    self.stdout,
            "stderr":    self.stderr,
            "timed_out": self.timed_out,
        })
    }
}
