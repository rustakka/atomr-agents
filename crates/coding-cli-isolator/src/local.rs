//! Host-process isolator.
//!
//! Headless: `tokio::process::Command` with piped stdin/stdout/stderr.
//! Interactive: `portable_pty` master + child with both stdin/stdout
//! flowing through one PTY channel.

use std::process::Stdio;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, Notify};

use atomr_agents_coding_cli_core::CliCommand;

use crate::error::IsolatorError;
use crate::handle::{ExitStatus, IsolationOpts, ProcessHandle};
use crate::pty_bridge::{self, PtyBridge};
use crate::traits::Isolator;

#[derive(Debug, Default, Clone)]
pub struct LocalIsolator;

impl LocalIsolator {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Isolator for LocalIsolator {
    fn name(&self) -> &str {
        "local"
    }

    async fn spawn(
        &self,
        cmd: CliCommand,
        opts: IsolationOpts,
    ) -> Result<Box<dyn ProcessHandle>, IsolatorError> {
        if cmd.allocate_pty {
            spawn_pty(cmd, opts).map(|h| Box::new(h) as Box<dyn ProcessHandle>)
        } else {
            spawn_pipes(cmd, opts).await.map(|h| Box::new(h) as Box<dyn ProcessHandle>)
        }
    }
}

// --------------------------------------------------------------------
// Headless (pipes) backend
// --------------------------------------------------------------------

struct PipedHandle {
    stdout_rx: Option<mpsc::Receiver<Vec<u8>>>,
    stderr_rx: Option<mpsc::Receiver<Vec<u8>>>,
    stdin_tx: Option<mpsc::Sender<Vec<u8>>>,
    child: Arc<Mutex<Option<tokio::process::Child>>>,
    cached_status: Arc<Mutex<Option<ExitStatus>>>,
    notify_exit: Arc<Notify>,
}

async fn spawn_pipes(cmd: CliCommand, opts: IsolationOpts) -> Result<PipedHandle, IsolatorError> {
    let mut command = tokio::process::Command::new(&cmd.program);
    command.args(&cmd.args);
    command.current_dir(&cmd.workdir);
    for (k, v) in &cmd.env {
        command.env(k, v);
    }
    command.stdin(Stdio::piped());
    command.stdout(if opts.capture_stdout { Stdio::piped() } else { Stdio::null() });
    command.stderr(if opts.capture_stderr { Stdio::piped() } else { Stdio::null() });
    let mut child = command
        .spawn()
        .map_err(|e| IsolatorError::Spawn(format!("{}: {e}", cmd.program.display())))?;

    let stdout_rx = if opts.capture_stdout {
        Some(pump_lines(child.stdout.take().expect("piped"), 8192))
    } else {
        None
    };
    let stderr_rx = if opts.capture_stderr {
        Some(pump_lines(child.stderr.take().expect("piped"), 8192))
    } else {
        None
    };
    let stdin_tx = child.stdin.take().map(pump_writes);

    Ok(PipedHandle {
        stdout_rx,
        stderr_rx,
        stdin_tx,
        child: Arc::new(Mutex::new(Some(child))),
        cached_status: Arc::new(Mutex::new(None)),
        notify_exit: Arc::new(Notify::new()),
    })
}

fn pump_lines<R>(reader: R, _buf_size: usize) -> mpsc::Receiver<Vec<u8>>
where
    R: tokio::io::AsyncRead + Send + Unpin + 'static,
{
    let (tx, rx) = mpsc::channel::<Vec<u8>>(256);
    tokio::spawn(async move {
        let mut br = BufReader::new(reader);
        let mut line = String::new();
        loop {
            line.clear();
            match br.read_line(&mut line).await {
                Ok(0) => break,
                Ok(_) => {
                    if tx.send(line.as_bytes().to_vec()).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    rx
}

fn pump_writes<W>(mut writer: W) -> mpsc::Sender<Vec<u8>>
where
    W: tokio::io::AsyncWrite + Send + Unpin + 'static,
{
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);
    tokio::spawn(async move {
        while let Some(buf) = rx.recv().await {
            if writer.write_all(&buf).await.is_err() {
                break;
            }
            let _ = writer.flush().await;
        }
    });
    tx
}

#[async_trait]
impl ProcessHandle for PipedHandle {
    fn take_stdout(&mut self) -> Option<mpsc::Receiver<Vec<u8>>> {
        self.stdout_rx.take()
    }
    fn take_stderr(&mut self) -> Option<mpsc::Receiver<Vec<u8>>> {
        self.stderr_rx.take()
    }
    fn take_stdin(&mut self) -> Option<mpsc::Sender<Vec<u8>>> {
        self.stdin_tx.take()
    }
    fn is_pty(&self) -> bool {
        false
    }
    async fn resize_pty(&mut self, _: u16, _: u16) -> Result<(), IsolatorError> {
        Err(IsolatorError::Unsupported("resize on piped stdin process"))
    }
    async fn kill(&mut self) -> Result<(), IsolatorError> {
        let mut guard = self.child.lock();
        if let Some(child) = guard.as_mut() {
            child.start_kill().map_err(IsolatorError::Io)?;
        }
        Ok(())
    }
    async fn wait(&mut self) -> Result<ExitStatus, IsolatorError> {
        if let Some(cached) = *self.cached_status.lock() {
            return Ok(cached);
        }
        let child_slot = self.child.clone();
        let cached = self.cached_status.clone();
        let notify = self.notify_exit.clone();

        // Drop the guard before awaiting to keep this future `Send`.
        let taken = { child_slot.lock().take() };
        let mut child = match taken {
            Some(c) => c,
            None => {
                notify.notified().await;
                return cached.lock().ok_or(IsolatorError::AlreadyExited);
            }
        };
        let status = child.wait().await.map_err(IsolatorError::Io)?;
        let exit = ExitStatus::from_code(status.code().unwrap_or(-1));
        *cached.lock() = Some(exit);
        notify.notify_waiters();
        Ok(exit)
    }
}


// --------------------------------------------------------------------
// Interactive (PTY) backend
// --------------------------------------------------------------------

struct PtyHandle {
    bridge: PtyBridge,
    cached_status: Arc<Mutex<Option<ExitStatus>>>,
}

fn spawn_pty(cmd: CliCommand, _opts: IsolationOpts) -> Result<PtyHandle, IsolatorError> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize { cols: 120, rows: 32, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| IsolatorError::Pty(format!("openpty: {e}")))?;
    let mut builder = CommandBuilder::new(cmd.program.as_os_str());
    for a in &cmd.args {
        builder.arg(a);
    }
    for (k, v) in &cmd.env {
        builder.env(k, v);
    }
    builder.cwd(cmd.workdir.as_os_str());

    let child = pair
        .slave
        .spawn_command(builder)
        .map_err(|e| IsolatorError::Pty(format!("spawn_command: {e}")))?;
    drop(pair.slave);

    let bridge = pty_bridge::spawn_pty_bridge(pair.master, child)?;
    Ok(PtyHandle {
        bridge,
        cached_status: Arc::new(Mutex::new(None)),
    })
}

#[async_trait]
impl ProcessHandle for PtyHandle {
    fn take_stdout(&mut self) -> Option<mpsc::Receiver<Vec<u8>>> {
        self.bridge.stdout_rx.take()
    }
    fn take_stderr(&mut self) -> Option<mpsc::Receiver<Vec<u8>>> {
        None
    }
    fn take_stdin(&mut self) -> Option<mpsc::Sender<Vec<u8>>> {
        self.bridge.stdin_tx.take()
    }
    fn is_pty(&self) -> bool {
        true
    }
    async fn resize_pty(&mut self, cols: u16, rows: u16) -> Result<(), IsolatorError> {
        pty_bridge::resize(&self.bridge.master, cols, rows)
    }
    async fn kill(&mut self) -> Result<(), IsolatorError> {
        pty_bridge::kill(&self.bridge.child)
    }
    async fn wait(&mut self) -> Result<ExitStatus, IsolatorError> {
        if let Some(cached) = *self.cached_status.lock() {
            return Ok(cached);
        }
        let status = pty_bridge::wait(self.bridge.child.clone()).await?;
        *self.cached_status.lock() = Some(status);
        Ok(status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_echo_headless() {
        let cmd = CliCommand::new("/bin/sh", std::env::temp_dir())
            .arg("-c")
            .arg("echo hello");
        let iso = LocalIsolator::new();
        let mut h = iso
            .spawn(
                cmd,
                IsolationOpts {
                    capture_stdout: true,
                    capture_stderr: false,
                    grace: None,
                },
            )
            .await
            .unwrap();
        let mut rx = h.take_stdout().unwrap();
        let first = rx.recv().await.unwrap();
        assert!(String::from_utf8_lossy(&first).contains("hello"));
        let status = h.wait().await.unwrap();
        assert!(status.success);
    }

    #[tokio::test]
    async fn local_pty_echo() {
        let cmd = CliCommand::new("/bin/sh", std::env::temp_dir())
            .arg("-c")
            .arg("printf 'pty-ok\\n'; sleep 0.05")
            .with_pty();
        let iso = LocalIsolator::new();
        let mut h = iso.spawn(cmd, IsolationOpts::default()).await.unwrap();
        assert!(h.is_pty());
        let mut rx = h.take_stdout().unwrap();
        // Drain until we see the marker or 1s elapses.
        let mut buf = Vec::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await {
                Ok(Some(chunk)) => {
                    buf.extend_from_slice(&chunk);
                    if String::from_utf8_lossy(&buf).contains("pty-ok") {
                        break;
                    }
                }
                _ => continue,
            }
        }
        assert!(String::from_utf8_lossy(&buf).contains("pty-ok"));
        let _ = h.wait().await;
    }
}
