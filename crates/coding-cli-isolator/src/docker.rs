//! Docker isolator backend (via bollard).
//!
//! Creates a fresh container per spawn, attaches to its stdin/stdout/
//! stderr (or a TTY for interactive), and bridges those streams to the
//! same `ProcessHandle` channel surface as the local backend. Bind
//! mounts the request's `workdir` into the container.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use bollard::container::{
    AttachContainerOptions, AttachContainerResults, Config as ContainerConfig,
    CreateContainerOptions, KillContainerOptions, LogOutput, RemoveContainerOptions,
    StartContainerOptions, WaitContainerOptions,
};
use bollard::models::{ContainerWaitResponse, HostConfig, Mount, MountTypeEnum};
use bollard::Docker;
use futures_util::{StreamExt, TryStreamExt};
use parking_lot::Mutex;
use tokio::sync::mpsc;

use atomr_agents_coding_cli_core::CliCommand;

use crate::error::IsolatorError;
use crate::handle::{ExitStatus, IsolationOpts, ProcessHandle};
use crate::traits::Isolator;

const CHANNEL_CAPACITY: usize = 256;

/// Configuration for `DockerIsolator`.
#[derive(Debug, Clone)]
pub struct DockerIsolatorConfig {
    /// Docker image to use for every spawn.
    pub image: String,
    /// Additional host→container bind mounts on top of the workdir.
    pub extra_mounts: Vec<DockerMount>,
    /// Environment variables to set inside the container.
    pub env: BTreeMap<String, String>,
    /// Network mode (e.g. `bridge`, `none`). `None` = bridge.
    pub network: Option<String>,
    /// Container path the workdir is mounted at. Defaults to
    /// `/workspace`.
    pub workdir_in_container: PathBuf,
    /// Auto-remove the container after exit.
    pub auto_remove: bool,
}

impl DockerIsolatorConfig {
    pub fn new(image: impl Into<String>) -> Self {
        Self {
            image: image.into(),
            extra_mounts: Vec::new(),
            env: BTreeMap::new(),
            network: None,
            workdir_in_container: PathBuf::from("/workspace"),
            auto_remove: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DockerMount {
    pub host_path: PathBuf,
    pub container_path: PathBuf,
    pub read_only: bool,
}

pub struct DockerIsolator {
    docker: Docker,
    cfg: DockerIsolatorConfig,
}

impl DockerIsolator {
    /// Connect to the local Docker daemon (Unix socket or `DOCKER_HOST`).
    pub fn local(cfg: DockerIsolatorConfig) -> Result<Self, IsolatorError> {
        let docker = Docker::connect_with_local_defaults()
            .map_err(|e| IsolatorError::Docker(format!("connect: {e}")))?;
        Ok(Self { docker, cfg })
    }

    pub fn with_docker(docker: Docker, cfg: DockerIsolatorConfig) -> Self {
        Self { docker, cfg }
    }
}

#[async_trait]
impl Isolator for DockerIsolator {
    fn name(&self) -> &str {
        "docker"
    }

    async fn spawn(
        &self,
        cmd: CliCommand,
        opts: IsolationOpts,
    ) -> Result<Box<dyn ProcessHandle>, IsolatorError> {
        let tty = cmd.allocate_pty;
        let work_in_container = self.cfg.workdir_in_container.clone();

        // Compose the in-container command.
        let mut argv: Vec<String> = vec![cmd.program.to_string_lossy().into_owned()];
        for a in &cmd.args {
            argv.push(a.to_string_lossy().into_owned());
        }

        // Mounts: workdir + extras.
        let mut mounts: Vec<Mount> = Vec::with_capacity(1 + self.cfg.extra_mounts.len());
        mounts.push(Mount {
            target: Some(work_in_container.to_string_lossy().into_owned()),
            source: Some(cmd.workdir.to_string_lossy().into_owned()),
            typ: Some(MountTypeEnum::BIND),
            read_only: Some(false),
            ..Default::default()
        });
        for m in &self.cfg.extra_mounts {
            mounts.push(Mount {
                target: Some(m.container_path.to_string_lossy().into_owned()),
                source: Some(m.host_path.to_string_lossy().into_owned()),
                typ: Some(MountTypeEnum::BIND),
                read_only: Some(m.read_only),
                ..Default::default()
            });
        }

        let mut env_vec: Vec<String> = Vec::new();
        for (k, v) in self.cfg.env.iter().chain(cmd.env.iter()) {
            env_vec.push(format!("{k}={v}"));
        }

        let host_config = HostConfig {
            mounts: Some(mounts),
            auto_remove: Some(self.cfg.auto_remove),
            network_mode: self.cfg.network.clone(),
            ..Default::default()
        };

        let config = ContainerConfig::<String> {
            image: Some(self.cfg.image.clone()),
            cmd: Some(argv),
            env: Some(env_vec),
            working_dir: Some(work_in_container.to_string_lossy().into_owned()),
            attach_stdin: Some(true),
            attach_stdout: Some(opts.capture_stdout || tty),
            attach_stderr: Some(opts.capture_stderr || tty),
            open_stdin: Some(true),
            stdin_once: Some(false),
            tty: Some(tty),
            host_config: Some(host_config),
            ..Default::default()
        };

        let create = self
            .docker
            .create_container(None::<CreateContainerOptions<String>>, config)
            .await?;
        let container_id = create.id;

        let attach_opts = AttachContainerOptions::<String> {
            stdin: Some(true),
            stdout: Some(opts.capture_stdout || tty),
            stderr: Some(opts.capture_stderr || tty),
            stream: Some(true),
            logs: Some(false),
            detach_keys: None,
        };
        let AttachContainerResults { output, input } = self
            .docker
            .attach_container(&container_id, Some(attach_opts))
            .await?;

        self.docker
            .start_container(&container_id, None::<StartContainerOptions<String>>)
            .await?;

        // Set up channels and pumps.
        let (stdout_tx, stdout_rx) = mpsc::channel::<Vec<u8>>(CHANNEL_CAPACITY);
        let (stderr_tx, stderr_rx) = mpsc::channel::<Vec<u8>>(CHANNEL_CAPACITY);
        let (stdin_tx, mut stdin_rx_for_pump) = mpsc::channel::<Vec<u8>>(CHANNEL_CAPACITY);

        let docker_for_wait = self.docker.clone();
        let cid_for_wait = container_id.clone();
        let cached_status: Arc<Mutex<Option<ExitStatus>>> = Arc::new(Mutex::new(None));

        // Output pump: bollard yields `LogOutput` items (StdOut/StdErr
        // variants, or Console when TTY). Forward each to the right
        // channel.
        tokio::spawn(async move {
            let mut s = output.boxed();
            while let Some(item) = s.next().await {
                match item {
                    Ok(LogOutput::StdOut { message }) | Ok(LogOutput::Console { message }) => {
                        if stdout_tx.send(message.to_vec()).await.is_err() {
                            break;
                        }
                    }
                    Ok(LogOutput::StdErr { message }) => {
                        if stderr_tx.send(message.to_vec()).await.is_err() {
                            break;
                        }
                    }
                    Ok(LogOutput::StdIn { .. }) => continue,
                    Err(_) => break,
                }
            }
        });

        // Input pump: drain mpsc → bollard `AsyncWrite`.
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let mut input = input;
            while let Some(chunk) = stdin_rx_for_pump.recv().await {
                if input.write_all(&chunk).await.is_err() {
                    break;
                }
                let _ = input.flush().await;
            }
        });

        Ok(Box::new(DockerProcessHandle {
            docker: self.docker.clone(),
            container_id,
            tty,
            stdout_rx: Some(stdout_rx),
            stderr_rx: Some(stderr_rx),
            stdin_tx: Some(stdin_tx),
            cached_status,
            _wait_seed: (docker_for_wait, cid_for_wait),
        }) as Box<dyn ProcessHandle>)
    }
}

struct DockerProcessHandle {
    docker: Docker,
    container_id: String,
    tty: bool,
    stdout_rx: Option<mpsc::Receiver<Vec<u8>>>,
    stderr_rx: Option<mpsc::Receiver<Vec<u8>>>,
    stdin_tx: Option<mpsc::Sender<Vec<u8>>>,
    cached_status: Arc<Mutex<Option<ExitStatus>>>,
    _wait_seed: (Docker, String),
}

#[async_trait]
impl ProcessHandle for DockerProcessHandle {
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
        self.tty
    }
    async fn resize_pty(&mut self, cols: u16, rows: u16) -> Result<(), IsolatorError> {
        if !self.tty {
            return Err(IsolatorError::Unsupported("resize on non-tty container"));
        }
        self.docker
            .resize_container_tty(
                &self.container_id,
                bollard::container::ResizeContainerTtyOptions {
                    height: rows,
                    width: cols,
                },
            )
            .await?;
        Ok(())
    }
    async fn kill(&mut self) -> Result<(), IsolatorError> {
        let _ = self
            .docker
            .kill_container(
                &self.container_id,
                Some(KillContainerOptions { signal: "SIGTERM" }),
            )
            .await;
        // Best-effort remove (auto_remove handles success path).
        let _ = self
            .docker
            .remove_container(
                &self.container_id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;
        Ok(())
    }
    async fn wait(&mut self) -> Result<ExitStatus, IsolatorError> {
        if let Some(cached) = *self.cached_status.lock() {
            return Ok(cached);
        }
        let mut s = self
            .docker
            .wait_container(&self.container_id, None::<WaitContainerOptions<String>>);
        let mut last: Option<ContainerWaitResponse> = None;
        while let Some(item) = s.try_next().await.transpose() {
            match item {
                Ok(r) => last = Some(r),
                Err(e) => return Err(IsolatorError::Docker(e.to_string())),
            }
        }
        let code = last.and_then(|r| Some(r.status_code as i32)).unwrap_or(-1);
        let status = ExitStatus::from_code(code);
        *self.cached_status.lock() = Some(status);
        Ok(status)
    }
}
