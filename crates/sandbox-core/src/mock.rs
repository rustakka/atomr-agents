//! `MockBackend` — an in-memory, deterministic backend.
//!
//! Lives in `-core` (like `MockWebSearch` in `web-search-core`) so every
//! downstream crate — tool, harness, pyo3 — can be unit-tested cross-platform
//! without Docker or KVM. It records file writes in memory, returns synthetic
//! exec output, and implements snapshot/fork by cloning its in-memory state.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use parking_lot::Mutex;

use crate::backend::{SandboxBackend, SandboxHandle};
use crate::error::{Result, SandboxError};
use crate::exit::ExitStatus;
use crate::id::{ExecId, SandboxId, SnapshotId};
use crate::request::{CreateSandbox, ExecRequest, ExecResult, SandboxInfo};

/// In-memory deterministic backend. Always [`available`](SandboxBackend::available).
#[derive(Debug, Default, Clone)]
pub struct MockBackend;

impl MockBackend {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl SandboxBackend for MockBackend {
    fn name(&self) -> &str {
        "mock"
    }

    async fn available(&self) -> bool {
        true
    }

    async fn create(&self, req: CreateSandbox) -> Result<Box<dyn SandboxHandle>> {
        let info = SandboxInfo {
            id: SandboxId::new(),
            profile: req.profile,
            budget: req.effective_budget(),
            backend: "mock".into(),
            boot_ms: 0,
            forked_from: req.from_snapshot.clone(),
            created_at: Utc::now(),
        };
        Ok(Box::new(MockHandle {
            info,
            fs: Arc::new(Mutex::new(BTreeMap::new())),
        }))
    }
}

/// A live mock sandbox. The in-memory filesystem is `Arc`-shared so `fork`
/// can clone it cheaply.
#[derive(Debug, Clone)]
pub struct MockHandle {
    info: SandboxInfo,
    fs: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
}

#[async_trait]
impl SandboxHandle for MockHandle {
    fn info(&self) -> &SandboxInfo {
        &self.info
    }

    async fn exec(&self, req: ExecRequest) -> Result<ExecResult> {
        // Backend invariant: a profile only runs the languages it ships. Shared
        // guard so the rule + message live in one place (profile.rs).
        self.info.profile.ensure_supports(req.language)?;
        let now = Utc::now();
        let stdout = format!(
            "[mock:{:?}] ran {} bytes of code with {} dependency/-ies",
            req.language,
            req.code.len(),
            req.dependencies.len()
        );
        Ok(ExecResult {
            exec_id: ExecId::new(),
            exit: ExitStatus::ok(),
            stdout,
            stderr: String::new(),
            started_at: now,
            ended_at: now,
            timed_out: false,
        })
    }

    async fn write_file(&self, path: &str, bytes: &[u8]) -> Result<()> {
        self.fs.lock().insert(path.to_string(), bytes.to_vec());
        Ok(())
    }

    async fn read_file(&self, path: &str) -> Result<Vec<u8>> {
        self.fs
            .lock()
            .get(path)
            .cloned()
            .ok_or_else(|| SandboxError::Guest(format!("no such file: {path}")))
    }

    async fn snapshot(&self) -> Result<SnapshotId> {
        Ok(SnapshotId::new())
    }

    async fn fork(&self) -> Result<Box<dyn SandboxHandle>> {
        // Low-fidelity stand-in for a Firecracker COW fork: clone the in-mem
        // filesystem into a fresh sandbox id, recording the snapshot lineage.
        let mut info = self.info.clone();
        info.id = SandboxId::new();
        info.forked_from = Some(SnapshotId::new());
        let fs = Arc::new(Mutex::new(self.fs.lock().clone()));
        Ok(Box::new(MockHandle { info, fs }))
    }

    async fn destroy(&self) -> Result<()> {
        Ok(())
    }
}
