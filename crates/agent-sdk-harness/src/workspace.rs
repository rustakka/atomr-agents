//! Per-session sandbox workspaces (Pattern C). Gated behind the `sandbox`
//! feature.
//!
//! When `spec.workspace.enabled`, each interactive session — and each headless
//! run — gets its own isolated sandbox (microVM/container) as a disposable
//! workspace: the `.claude/` projection is staged into it, the agent's
//! exec/file actions are routed there via the in-process `run_in_sandbox` tool
//! (host `Bash`/`Write`/`Edit` disabled), and the workspace is discarded — or
//! snapshotted to the warm pool — when the session closes.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use atomr_agents_sandbox_core::{SandboxBackendSel, SandboxId, SandboxProfile};

/// Harness-spec knob controlling per-session sandbox workspaces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxWorkspaceConfig {
    /// Master switch. When `false`, the harness behaves exactly as without the
    /// feature: host-filesystem projection, no sandbox, no containment.
    #[serde(default)]
    pub enabled: bool,

    /// Toolchain profile for the workspace sandbox.
    #[serde(default = "default_ws_profile")]
    pub profile: SandboxProfile,

    /// Backend selection. `None` resolves to [`SandboxBackendSel::Auto`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<SandboxBackendSel>,

    /// What happens to the workspace when the session closes.
    #[serde(default)]
    pub on_close: WorkspaceDisposition,

    /// Prefer a warm fork from the snapshot pool over a cold create.
    #[serde(default = "default_true")]
    pub reuse_warm: bool,
}

impl Default for SandboxWorkspaceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            profile: default_ws_profile(),
            backend: None,
            on_close: WorkspaceDisposition::default(),
            reuse_warm: true,
        }
    }
}

fn default_ws_profile() -> SandboxProfile {
    SandboxProfile::PythonAndNpm
}
fn default_true() -> bool {
    true
}

/// Teardown policy for a session workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceDisposition {
    /// Destroy the sandbox on close — ephemeral, no residue (the default).
    #[default]
    Discard,
    /// Snapshot into the warm pool (for faster future starts), then destroy
    /// the live instance.
    Snapshot,
}

/// A live per-session workspace: the sandbox backing it plus its teardown
/// policy. Held in the [`WorkspaceRegistry`] for the session's lifetime.
#[derive(Debug, Clone)]
pub struct SessionWorkspace {
    pub sandbox_id: SandboxId,
    pub profile: SandboxProfile,
    pub on_close: WorkspaceDisposition,
}

/// Maps a session id → its live workspace. Mirrors `SessionRegistry`'s
/// `parking_lot::RwLock` shape so the base crate needs no extra `dashmap` dep.
#[derive(Default, Clone)]
pub struct WorkspaceRegistry {
    inner: Arc<RwLock<HashMap<String, SessionWorkspace>>>,
}

impl WorkspaceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, key: String, ws: SessionWorkspace) {
        self.inner.write().insert(key, ws);
    }

    /// Remove and return a workspace (for teardown on session close).
    pub fn take(&self, key: &str) -> Option<SessionWorkspace> {
        self.inner.write().remove(key)
    }

    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.read().is_empty()
    }
}
