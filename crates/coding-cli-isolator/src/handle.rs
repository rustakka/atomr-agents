//! `ProcessHandle` — the uniform handle every isolator returns.

use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::error::IsolatorError;

/// Per-spawn options the harness can tweak independently of the
/// command itself.
#[derive(Debug, Clone, Default)]
pub struct IsolationOpts {
    /// Capture stdout (NDJSON for headless). Always `true` in v1.
    pub capture_stdout: bool,
    /// Capture stderr.
    pub capture_stderr: bool,
    /// Wait this long after `kill` before forcing termination. None =
    /// platform default.
    pub grace: Option<Duration>,
}

/// Final exit status from a spawned process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitStatus {
    pub code: Option<i32>,
    pub success: bool,
}

impl ExitStatus {
    pub fn ok() -> Self {
        Self { code: Some(0), success: true }
    }
    pub fn from_code(code: i32) -> Self {
        Self { code: Some(code), success: code == 0 }
    }
}

/// Uniform handle to a running CLI process — host or container.
///
/// The handle owns one set of byte channels:
///
/// * `stdout_rx` / `stderr_rx`: lines (or chunks) emitted by the
///   process. For PTY-allocated processes, all output flows through
///   `stdout_rx` (stderr is multiplexed by the kernel).
/// * `stdin_tx`: writes from the harness back into the process.
///   `None` for processes spawned without an input channel.
///
/// All channels are tokio mpsc; the isolator background tasks
/// translate platform-specific I/O to these channels.
#[async_trait]
pub trait ProcessHandle: Send {
    /// Take ownership of the stdout byte stream. Returns `None` once
    /// already taken.
    fn take_stdout(&mut self) -> Option<mpsc::Receiver<Vec<u8>>>;

    /// Take ownership of the stderr byte stream.
    fn take_stderr(&mut self) -> Option<mpsc::Receiver<Vec<u8>>>;

    /// Take ownership of the stdin sender.
    fn take_stdin(&mut self) -> Option<mpsc::Sender<Vec<u8>>>;

    /// `true` if the process was spawned with a PTY (interactive).
    fn is_pty(&self) -> bool;

    /// Resize the PTY window. Errors with `Unsupported` if not a PTY
    /// process.
    async fn resize_pty(&mut self, cols: u16, rows: u16) -> Result<(), IsolatorError>;

    /// Send SIGTERM (and SIGKILL after grace).
    async fn kill(&mut self) -> Result<(), IsolatorError>;

    /// Wait for the process to exit. Idempotent — repeated calls
    /// after exit return the cached status.
    async fn wait(&mut self) -> Result<ExitStatus, IsolatorError>;
}
