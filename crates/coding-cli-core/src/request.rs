//! Inputs accepted by the coding-cli harness.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::projection::ConceptProjection;
use crate::vendor::CliVendorKind;

macro_rules! id_newtype {
    ($name:ident, $prefix:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new() -> Self {
                Self(format!("{}-{}", $prefix, Uuid::new_v4()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }
        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_owned())
            }
        }
    };
}

id_newtype!(CliRunId, "cli-run");
id_newtype!(CliSessionId, "cli-sess");

/// Whether the harness should run the CLI headlessly (parse structured
/// events) or interactively (bridge a tmux session to a browser).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    Headless,
    Interactive,
}

impl Default for RunMode {
    fn default() -> Self {
        RunMode::Headless
    }
}

/// Where the CLI process runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IsolationSpec {
    /// Spawn the CLI directly on the host.
    Local,
    /// Spawn the CLI inside a Docker container.
    Docker {
        /// Image reference (e.g. `atomr-agents/coding-cli-claude:latest`).
        image: String,
        /// Host→container bind mounts. The harness always mounts the
        /// project workdir; additional mounts (e.g. credential files)
        /// go here.
        #[serde(default)]
        mounts: Vec<DockerMount>,
        /// Environment variables to set inside the container.
        #[serde(default)]
        env: BTreeMap<String, String>,
        /// Network mode (default: `bridge`). Use `none` to fully isolate.
        #[serde(default)]
        network: Option<String>,
    },
}

impl Default for IsolationSpec {
    fn default() -> Self {
        IsolationSpec::Local
    }
}

/// One bind mount entry for `IsolationSpec::Docker`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerMount {
    pub host_path: PathBuf,
    pub container_path: PathBuf,
    #[serde(default)]
    pub read_only: bool,
}

/// Budgets shared across the run. Mirrors the budget plumbing in the
/// rest of the framework but stays decoupled from
/// `atomr-agents-core::TokenBudget` so this crate remains lightweight.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BudgetSpec {
    /// Wall-clock cap for the run.
    #[serde(default, with = "duration_secs_opt", skip_serializing_if = "Option::is_none")]
    pub wall_clock: Option<Duration>,
    /// Total token cap across all CLI calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    /// Optional money cap in micro-USD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_money_micro_usd: Option<u64>,
}

/// Uniform request the harness accepts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliRequest {
    /// Which adapter to dispatch to.
    pub vendor: CliVendorKind,

    /// Headless or interactive.
    #[serde(default)]
    pub mode: RunMode,

    /// Free-text prompt fed to the CLI. For interactive runs this may
    /// be empty (the operator drives the session).
    #[serde(default)]
    pub prompt: String,

    /// Working directory the CLI executes against. The harness sets
    /// this as `cwd` (for `Local`) or bind-mounts it (for `Docker`).
    pub workdir: PathBuf,

    /// Model id override (vendor-specific string). `None` lets the CLI
    /// pick its own default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Vendor-specific allow-list of tool names. Empty = no restriction.
    #[serde(default)]
    pub allowed_tools: Vec<String>,

    /// Resume an existing CLI session id if the vendor supports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_session: Option<String>,

    /// Concept projection — atomr Skills/Persona/Policy/toolsets the
    /// vendor adapter materializes to on-disk config before the run.
    #[serde(default)]
    pub project: ConceptProjection,

    /// Where the CLI runs.
    #[serde(default)]
    pub isolation: IsolationSpec,

    /// Run budget caps.
    #[serde(default)]
    pub budget: BudgetSpec,

    /// Free-form metadata the caller can stash on the request. Echoed
    /// back on the `CliResult`.
    #[serde(default)]
    pub metadata: BTreeMap<String, serde_json::Value>,
}

impl CliRequest {
    /// Shortcut for the common case.
    pub fn new(vendor: CliVendorKind, workdir: impl Into<PathBuf>, prompt: impl Into<String>) -> Self {
        Self {
            vendor,
            mode: RunMode::Headless,
            prompt: prompt.into(),
            workdir: workdir.into(),
            model: None,
            allowed_tools: Vec::new(),
            resume_session: None,
            project: ConceptProjection::default(),
            isolation: IsolationSpec::Local,
            budget: BudgetSpec::default(),
            metadata: BTreeMap::new(),
        }
    }

    pub fn with_mode(mut self, mode: RunMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn with_isolation(mut self, isolation: IsolationSpec) -> Self {
        self.isolation = isolation;
        self
    }

    pub fn with_project(mut self, project: ConceptProjection) -> Self {
        self.project = project;
        self
    }
}

mod duration_secs_opt {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        match d {
            Some(d) => s.serialize_some(&d.as_secs()),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        let v: Option<u64> = Option::deserialize(d)?;
        Ok(v.map(Duration::from_secs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips_json() {
        let r = CliRequest::new(CliVendorKind::Claude, "/tmp/workdir", "list files")
            .with_model("claude-sonnet-4-6")
            .with_mode(RunMode::Headless);
        let j = serde_json::to_string(&r).unwrap();
        let back: CliRequest = serde_json::from_str(&j).unwrap();
        assert_eq!(back.vendor, CliVendorKind::Claude);
        assert_eq!(back.model.as_deref(), Some("claude-sonnet-4-6"));
        assert_eq!(back.mode, RunMode::Headless);
    }

    #[test]
    fn isolation_default_is_local() {
        assert!(matches!(IsolationSpec::default(), IsolationSpec::Local));
    }

    #[test]
    fn run_id_is_unique() {
        let a = CliRunId::new();
        let b = CliRunId::new();
        assert_ne!(a, b);
        assert!(a.as_str().starts_with("cli-run-"));
    }
}
