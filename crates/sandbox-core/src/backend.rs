//! The pluggable backend abstraction. [`SandboxBackend`] mirrors the
//! `Isolator` trait and [`SandboxHandle`] mirrors `ProcessHandle` from
//! `coding-cli-isolator`, extended with snapshot/fork for warm-start
//! branching.

use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};

use crate::error::Result;
use crate::id::SnapshotId;
use crate::request::{CreateSandbox, ExecRequest, ExecResult, SandboxInfo};

/// Provisions sandboxes in some execution environment (mock, Docker,
/// Firecracker, or a remote cluster). The harness holds one as
/// `Arc<dyn SandboxBackend>` and injects it, exactly like `Arc<dyn Isolator>`.
#[async_trait]
pub trait SandboxBackend: Send + Sync {
    /// Stable identifier used in logs and [`SandboxInfo::backend`].
    fn name(&self) -> &str;

    /// Whether this backend is usable on the current host (KVM present,
    /// Docker reachable, endpoint configured…). `Auto` resolution uses this.
    async fn available(&self) -> bool;

    /// Cold-boot a fresh sandbox, or warm-fork when `req.from_snapshot` is set.
    ///
    /// **Contract:** implementations MUST provision using
    /// [`req.effective_budget()`](CreateSandbox::effective_budget) and MUST NOT
    /// read `req.budget` directly — `effective_budget` re-applies the Rust
    /// 2 GB / 2 vCPU floor, which a raw `req.budget` can undercut. The harness
    /// also normalizes `req.budget` to the effective value before dispatch, so
    /// the floor is enforced even if an implementation forgets.
    async fn create(&self, req: CreateSandbox) -> Result<Box<dyn SandboxHandle>>;
}

/// A uniform handle to one live sandbox.
#[async_trait]
pub trait SandboxHandle: Send + Sync {
    fn info(&self) -> &SandboxInfo;

    /// Run code to completion, returning captured output.
    async fn exec(&self, req: ExecRequest) -> Result<ExecResult>;

    /// Streaming variant: stdout/stderr arrive as byte chunks while the exec
    /// runs, with the final [`ExecResult`] on a oneshot. The default impl
    /// buffers [`exec`](Self::exec) then emits once — backends that can truly
    /// stream (Firecracker via vsock) override this.
    async fn exec_streaming(&self, req: ExecRequest) -> Result<ExecStream> {
        let res = self.exec(req).await?;
        let (stdout_tx, stdout_rx) = mpsc::channel(16);
        let (stderr_tx, stderr_rx) = mpsc::channel(16);
        let (result_tx, result_rx) = oneshot::channel();
        if !res.stdout.is_empty() {
            let _ = stdout_tx.send(res.stdout.clone().into_bytes()).await;
        }
        if !res.stderr.is_empty() {
            let _ = stderr_tx.send(res.stderr.clone().into_bytes()).await;
        }
        let _ = result_tx.send(res);
        Ok(ExecStream { stdout_rx, stderr_rx, result: result_rx })
    }

    /// Write a file into the sandbox filesystem.
    ///
    /// **Contract:** implementations that map `path` onto a real host/guest
    /// filesystem MUST confine it to the sandbox root — reject or
    /// canonicalize-and-clamp absolute paths and `..` traversal (returning
    /// [`SandboxError::BudgetViolation`](crate::SandboxError::BudgetViolation)
    /// on escape) so untrusted code cannot read/write outside the sandbox.
    async fn write_file(&self, path: &str, bytes: &[u8]) -> Result<()>;

    /// Read a file back out of the sandbox filesystem. Same path-confinement
    /// contract as [`write_file`](Self::write_file).
    async fn read_file(&self, path: &str) -> Result<Vec<u8>>;

    /// Snapshot live memory + disk state. Backends without snapshot support
    /// return [`SandboxError::Unsupported`](crate::SandboxError::Unsupported).
    async fn snapshot(&self) -> Result<SnapshotId>;

    /// Fork a NEW sandbox from this one's current state (the <50 ms warm-fork
    /// path on Firecracker). Backs agent branch states.
    async fn fork(&self) -> Result<Box<dyn SandboxHandle>>;

    /// Tear down the sandbox and reclaim its resources. Idempotent — backends
    /// take `&self` (not `Box<Self>`) so handles stay `Arc`-shareable for the
    /// harness registry and PyO3 wrappers.
    async fn destroy(&self) -> Result<()>;
}

/// Live streaming channels for an exec — the same `mpsc<Vec<u8>>` surface as
/// `ProcessHandle`, plus a oneshot carrying the terminal result.
pub struct ExecStream {
    pub stdout_rx: mpsc::Receiver<Vec<u8>>,
    pub stderr_rx: mpsc::Receiver<Vec<u8>>,
    pub result: oneshot::Receiver<ExecResult>,
}
