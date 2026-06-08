//! Guest-agent core: the transport-agnostic request/response server, plus
//! confined file I/O and code execution. This is the substance of the in-VM
//! PID-1 daemon, written so it serves over *any* `AsyncRead + AsyncWrite`
//! (an in-memory pipe in tests, a real `AF_VSOCK` socket in the guest).
//!
//! The binary (`src/main.rs`) is a thin Linux-only shell that does PID-1 init,
//! binds vsock, and calls [`serve`] per connection.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tokio::io::{AsyncRead, AsyncWrite};

use atomr_agents_sandbox_proto::{
    read_frame, write_frame, GuestRequest, GuestResponse, Language, ProtoError,
};

pub mod exec;

/// Default in-guest working directory the sandbox filesystem is rooted at.
pub const DEFAULT_ROOT: &str = "/workspace";

/// Guest agent configuration.
#[derive(Debug, Clone)]
pub struct GuestConfig {
    /// Filesystem root; all file ops and exec cwd are confined here.
    pub root: PathBuf,
}

impl Default for GuestConfig {
    fn default() -> Self {
        Self { root: PathBuf::from(DEFAULT_ROOT) }
    }
}

impl GuestConfig {
    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

/// Reject a path that would escape the sandbox root, then join it under `root`.
fn resolve(root: &Path, path: &str) -> Result<PathBuf, String> {
    if path.starts_with('/') || path.split(['/', '\\']).any(|c| c == "..") {
        return Err(format!("path escapes sandbox root: {path}"));
    }
    Ok(root.join(path.trim_start_matches("./")))
}

async fn handle_write(root: &Path, path: &str, bytes: &[u8]) -> Result<(), String> {
    let full = resolve(root, path)?;
    if let Some(parent) = full.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("mkdir: {e}"))?;
    }
    tokio::fs::write(&full, bytes)
        .await
        .map_err(|e| format!("write: {e}"))
}

async fn handle_read(root: &Path, path: &str) -> Result<Vec<u8>, String> {
    let full = resolve(root, path)?;
    tokio::fs::read(&full).await.map_err(|e| format!("read: {e}"))
}

/// One exec job, bundling the fields of [`GuestRequest::Exec`].
struct ExecJob {
    exec_id: String,
    language: Language,
    code: String,
    dependencies: Vec<String>,
    stdin: Option<String>,
    env: Vec<(String, String)>,
    timeout_ms: Option<u64>,
}

async fn handle_exec<W>(writer: &mut W, root: &Path, job: ExecJob) -> Result<(), ProtoError>
where
    W: AsyncWrite + Unpin,
{
    let ExecJob { exec_id, language, code, dependencies, stdin, env, timeout_ms } = job;
    let env_map: HashMap<String, String> = env.into_iter().collect();
    let timeout = timeout_ms.map(std::time::Duration::from_millis);

    // Best-effort dependency install first.
    let mut dep_stderr: Vec<u8> = Vec::new();
    if let Some((prog, args)) = exec::deps_command(language, &dependencies) {
        match exec::run_capture(root, &prog, &args, &env_map, None, timeout).await {
            Ok(cap) => dep_stderr = cap.stderr,
            Err(e) => dep_stderr = e.into_bytes(),
        }
    }

    let (prog, args) = exec::code_command(language, &code, root);
    match exec::run_capture(root, &prog, &args, &env_map, stdin.as_deref(), timeout).await {
        Ok(cap) => {
            if !cap.stdout.is_empty() {
                write_frame(
                    writer,
                    &GuestResponse::ExecStdout { exec_id: exec_id.clone(), chunk: cap.stdout },
                )
                .await?;
            }
            let mut stderr = dep_stderr;
            stderr.extend_from_slice(&cap.stderr);
            if !stderr.is_empty() {
                write_frame(
                    writer,
                    &GuestResponse::ExecStderr { exec_id: exec_id.clone(), chunk: stderr },
                )
                .await?;
            }
            write_frame(
                writer,
                &GuestResponse::ExecDone {
                    exec_id,
                    exit_code: cap.exit_code,
                    timed_out: cap.timed_out,
                },
            )
            .await?;
        }
        Err(e) => {
            write_frame(writer, &GuestResponse::Error { message: e }).await?;
        }
    }
    Ok(())
}

/// Serve the guest protocol over one duplex connection (e.g. a vsock socket)
/// until the peer closes it or sends `Shutdown`.
pub async fn serve<S>(stream: S, config: &GuestConfig) -> Result<bool, ProtoError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (reader, writer) = tokio::io::split(stream);
    serve_rw(reader, writer, config).await
}

/// Serve the guest protocol over a separate reader and writer (e.g. the
/// process's stdin and stdout). Returns `Ok(true)` if the client asked the VM
/// to shut down.
pub async fn serve_rw<R, W>(
    mut reader: R,
    mut writer: W,
    config: &GuestConfig,
) -> Result<bool, ProtoError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let root = config.root.as_path();

    while let Some(req) = read_frame::<_, GuestRequest>(&mut reader).await? {
        match req {
            GuestRequest::Ping => {
                write_frame(&mut writer, &GuestResponse::Pong).await?;
            }
            GuestRequest::WriteFile { path, bytes } => {
                let resp = match handle_write(root, &path, &bytes).await {
                    Ok(()) => GuestResponse::FileWritten,
                    Err(message) => GuestResponse::Error { message },
                };
                write_frame(&mut writer, &resp).await?;
            }
            GuestRequest::ReadFile { path } => {
                let resp = match handle_read(root, &path).await {
                    Ok(bytes) => GuestResponse::FileRead { bytes },
                    Err(message) => GuestResponse::Error { message },
                };
                write_frame(&mut writer, &resp).await?;
            }
            GuestRequest::Exec {
                exec_id,
                language,
                code,
                dependencies,
                stdin,
                env,
                timeout_ms,
            } => {
                let job = ExecJob {
                    exec_id,
                    language,
                    code,
                    dependencies,
                    stdin,
                    env,
                    timeout_ms,
                };
                handle_exec(&mut writer, root, job).await?;
            }
            GuestRequest::Shutdown => {
                let _ = write_frame(&mut writer, &GuestResponse::Pong).await;
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_sandbox_proto::{GuestRequest, GuestResponse};

    /// Drive `serve` over an in-memory duplex: the test plays the host side.
    async fn with_server<F, Fut>(test: F)
    where
        F: FnOnce(tokio::io::DuplexStream) -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let dir = tempfile::tempdir().unwrap();
        let config = GuestConfig::with_root(dir.path());
        let (host, guest) = tokio::io::duplex(64 * 1024);
        let server = tokio::spawn(async move { serve(guest, &config).await });
        test(host).await;
        let _ = server.await;
    }

    #[tokio::test]
    async fn ping_pong() {
        with_server(|mut host| async move {
            write_frame(&mut host, &GuestRequest::Ping).await.unwrap();
            let resp: GuestResponse = read_frame(&mut host).await.unwrap().unwrap();
            assert_eq!(resp, GuestResponse::Pong);
        })
        .await;
    }

    #[tokio::test]
    async fn write_then_read_file() {
        with_server(|mut host| async move {
            write_frame(
                &mut host,
                &GuestRequest::WriteFile { path: "src/main.rs".into(), bytes: b"fn main(){}".to_vec() },
            )
            .await
            .unwrap();
            let r: GuestResponse = read_frame(&mut host).await.unwrap().unwrap();
            assert_eq!(r, GuestResponse::FileWritten);

            write_frame(&mut host, &GuestRequest::ReadFile { path: "src/main.rs".into() })
                .await
                .unwrap();
            let r: GuestResponse = read_frame(&mut host).await.unwrap().unwrap();
            assert_eq!(r, GuestResponse::FileRead { bytes: b"fn main(){}".to_vec() });
        })
        .await;
    }

    #[tokio::test]
    async fn path_traversal_rejected() {
        with_server(|mut host| async move {
            write_frame(
                &mut host,
                &GuestRequest::WriteFile { path: "../escape".into(), bytes: b"x".to_vec() },
            )
            .await
            .unwrap();
            let r: GuestResponse = read_frame(&mut host).await.unwrap().unwrap();
            assert!(matches!(r, GuestResponse::Error { .. }));
        })
        .await;
    }

    #[tokio::test]
    async fn read_missing_file_is_error() {
        with_server(|mut host| async move {
            write_frame(&mut host, &GuestRequest::ReadFile { path: "nope.txt".into() })
                .await
                .unwrap();
            let r: GuestResponse = read_frame(&mut host).await.unwrap().unwrap();
            assert!(matches!(r, GuestResponse::Error { .. }));
        })
        .await;
    }

    #[tokio::test]
    async fn shutdown_ends_the_session() {
        with_server(|mut host| async move {
            write_frame(&mut host, &GuestRequest::Shutdown).await.unwrap();
            let r: GuestResponse = read_frame(&mut host).await.unwrap().unwrap();
            assert_eq!(r, GuestResponse::Pong);
        })
        .await;
    }
}
