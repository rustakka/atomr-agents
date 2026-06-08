//! `DockerBackend` — the cross-platform "Insecure Dev Mode" sandbox backend.
//!
//! Containers share the host kernel, so this is **not** a security boundary
//! for untrusted code (that is what the Firecracker backend is for). It exists
//! so the sandbox surface runs end-to-end on a developer laptop with only
//! Docker Desktop — real Python / Bash / JavaScript / Rust execution, file
//! I/O, and commit-based snapshot/fork.
//!
//! Model: one long-lived container per sandbox (`sleep infinity`), with each
//! exec run via the Docker exec API inside it. Reuses the bollard machinery
//! proven in `coding-cli-isolator`.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine as _;
use bollard::container::{
    Config as ContainerConfig, CreateContainerOptions, DownloadFromContainerOptions,
    KillContainerOptions, LogOutput, RemoveContainerOptions, StartContainerOptions,
    UploadToContainerOptions,
};
use bollard::exec::{CreateExecOptions, StartExecOptions, StartExecResults};
use bollard::image::{CommitContainerOptions, CreateImageOptions};
use bollard::models::HostConfig;
use bollard::Docker;
use chrono::Utc;
use futures_util::StreamExt;
use std::io::Read as _;

use atomr_agents_sandbox_core::{
    CreateSandbox, ExecId, ExecRequest, ExecResult, ExitStatus, Language, SandboxBackend,
    SandboxBackendSel, SandboxError, SandboxHandle, SandboxId, SandboxInfo, SandboxProfile,
    SnapshotId,
};

const WORKDIR: &str = "/workspace";

fn map_docker(e: bollard::errors::Error) -> SandboxError {
    SandboxError::Backend(format!("docker: {e}"))
}

/// Reject paths that would escape the sandbox working directory.
fn confine(path: &str) -> Result<String, SandboxError> {
    if path.starts_with('/') || path.split(['/', '\\']).any(|c| c == "..") {
        return Err(SandboxError::BudgetViolation(format!(
            "path escapes sandbox root: {path}"
        )));
    }
    Ok(path.trim_start_matches("./").to_string())
}

/// Maps a profile to the Docker image used for it. The interpreted profiles map
/// to slim official images; Rust to the official toolchain image.
fn default_image(profile: SandboxProfile) -> &'static str {
    match profile {
        SandboxProfile::PythonOnly => "python:3.11-slim",
        SandboxProfile::NpmOnly => "node:20-slim",
        SandboxProfile::RustOnly => "rust:1-slim",
        // No single official image carries every toolchain; the combined
        // community image covers Python+Node. Rust under these profiles is
        // best-effort in Docker mode — the Firecracker backend is the real path.
        SandboxProfile::PythonAndNpm | SandboxProfile::FullStack => {
            "nikolaik/python-nodejs:latest"
        }
    }
}

/// Configuration for [`DockerBackend`].
#[derive(Debug, Clone)]
pub struct DockerBackendConfig {
    /// Docker network mode (`None` = default bridge; `Some("none")` for no net).
    pub network: Option<String>,
    /// Pull the image if it is not already present locally.
    pub pull_missing: bool,
    /// Per-profile image overrides.
    pub image_overrides: HashMap<SandboxProfile, String>,
}

impl Default for DockerBackendConfig {
    fn default() -> Self {
        Self { network: None, pull_missing: true, image_overrides: HashMap::new() }
    }
}

/// Docker-backed sandbox provisioner.
#[derive(Clone)]
pub struct DockerBackend {
    docker: Docker,
    config: DockerBackendConfig,
}

impl DockerBackend {
    /// Connect to the local Docker daemon (socket or `DOCKER_HOST`).
    pub fn local() -> Result<Self, SandboxError> {
        Self::with_config(DockerBackendConfig::default())
    }

    pub fn with_config(config: DockerBackendConfig) -> Result<Self, SandboxError> {
        let docker = Docker::connect_with_local_defaults()
            .map_err(|e| SandboxError::Backend(format!("docker connect: {e}")))?;
        Ok(Self { docker, config })
    }

    pub fn with_docker(docker: Docker, config: DockerBackendConfig) -> Self {
        Self { docker, config }
    }

    fn resolve_image(&self, req: &CreateSandbox) -> String {
        if let SandboxBackendSel::Docker { image: Some(img) } = &req.backend {
            return img.clone();
        }
        if let Some(img) = self.config.image_overrides.get(&req.profile) {
            return img.clone();
        }
        default_image(req.profile).to_string()
    }

    async fn ensure_image(&self, image: &str) -> Result<(), SandboxError> {
        if self.docker.inspect_image(image).await.is_ok() {
            return Ok(());
        }
        // The Docker pull API wants the repo and tag as separate fields; folding
        // the tag into `from_image` makes the daemon append `:latest`.
        let (from_image, tag) = match image.rsplit_once(':') {
            Some((repo, tag)) if !tag.contains('/') => (repo, tag),
            _ => (image, "latest"),
        };
        let mut stream = self.docker.create_image(
            Some(CreateImageOptions { from_image, tag, ..Default::default() }),
            None,
            None,
        );
        while let Some(item) = stream.next().await {
            item.map_err(map_docker)?;
        }
        Ok(())
    }

    async fn launch_container(&self, image: &str) -> Result<String, SandboxError> {
        let host_config = HostConfig {
            network_mode: self.config.network.clone(),
            auto_remove: Some(false),
            ..Default::default()
        };
        let config = ContainerConfig {
            image: Some(image.to_string()),
            // Keep the container alive so we can exec into it repeatedly.
            cmd: Some(vec!["sleep".to_string(), "infinity".to_string()]),
            working_dir: Some(WORKDIR.to_string()),
            host_config: Some(host_config),
            ..Default::default()
        };
        let created = self
            .docker
            .create_container(None::<CreateContainerOptions<String>>, config)
            .await
            .map_err(map_docker)?;
        self.docker
            .start_container(&created.id, None::<StartContainerOptions<String>>)
            .await
            .map_err(map_docker)?;
        Ok(created.id)
    }
}

#[async_trait]
impl SandboxBackend for DockerBackend {
    fn name(&self) -> &str {
        "docker"
    }

    async fn available(&self) -> bool {
        self.docker.ping().await.is_ok()
    }

    async fn create(&self, req: CreateSandbox) -> Result<Box<dyn SandboxHandle>, SandboxError> {
        let started = Instant::now();
        let image = self.resolve_image(&req);
        if self.config.pull_missing {
            self.ensure_image(&image).await?;
        }
        let container_id = self.launch_container(&image).await?;
        let info = SandboxInfo {
            id: SandboxId::new(),
            profile: req.profile,
            budget: req.effective_budget(),
            backend: "docker".into(),
            boot_ms: started.elapsed().as_millis() as u64,
            forked_from: req.from_snapshot.clone(),
            created_at: Utc::now(),
        };
        Ok(Box::new(DockerHandle {
            docker: self.docker.clone(),
            config: self.config.clone(),
            container_id,
            info,
        }))
    }
}

/// A live Docker-backed sandbox.
pub struct DockerHandle {
    docker: Docker,
    config: DockerBackendConfig,
    container_id: String,
    info: SandboxInfo,
}

impl DockerHandle {
    /// Run one argv inside the container, returning (stdout, stderr, code, timed_out).
    async fn run(
        &self,
        cmd: Vec<String>,
        env: &BTreeMap<String, String>,
        timeout: Option<Duration>,
    ) -> Result<(String, String, i64, bool), SandboxError> {
        let env_vec: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let exec = self
            .docker
            .create_exec(
                &self.container_id,
                CreateExecOptions {
                    cmd: Some(cmd),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    working_dir: Some(WORKDIR.to_string()),
                    env: if env_vec.is_empty() { None } else { Some(env_vec) },
                    ..Default::default()
                },
            )
            .await
            .map_err(map_docker)?;

        let start = self
            .docker
            .start_exec(&exec.id, None::<StartExecOptions>)
            .await
            .map_err(map_docker)?;

        let mut stdout: Vec<u8> = Vec::new();
        let mut stderr: Vec<u8> = Vec::new();
        let mut timed_out = false;

        if let StartExecResults::Attached { mut output, .. } = start {
            let drain = async {
                while let Some(item) = output.next().await {
                    match item {
                        Ok(LogOutput::StdOut { message }) => stdout.extend_from_slice(&message),
                        Ok(LogOutput::StdErr { message }) => stderr.extend_from_slice(&message),
                        Ok(LogOutput::Console { message }) => stdout.extend_from_slice(&message),
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
            };
            match timeout {
                Some(d) => {
                    if tokio::time::timeout(d, drain).await.is_err() {
                        timed_out = true;
                    }
                }
                None => drain.await,
            }
        }

        let inspect = self.docker.inspect_exec(&exec.id).await.map_err(map_docker)?;
        let code = inspect.exit_code.unwrap_or(if timed_out { 124 } else { -1 });
        Ok((
            String::from_utf8_lossy(&stdout).into_owned(),
            String::from_utf8_lossy(&stderr).into_owned(),
            code,
            timed_out,
        ))
    }
}

/// Dependency-install argv for a language, if any deps are requested.
fn deps_command(lang: Language, deps: &[String]) -> Option<Vec<String>> {
    if deps.is_empty() {
        return None;
    }
    match lang {
        Language::Python => {
            let mut c = vec!["pip".into(), "install".into(), "--quiet".into()];
            c.extend(deps.iter().cloned());
            Some(c)
        }
        Language::Js => {
            let mut c = vec!["npm".into(), "install".into(), "--silent".into()];
            c.extend(deps.iter().cloned());
            Some(c)
        }
        // cargo add needs a project; bash has no package manager here.
        Language::Rust | Language::Bash => None,
    }
}

/// Argv that runs the user code. Interpreted languages pass code as a literal
/// argv element (no shell, so no quoting hazard); Rust is base64-piped to a
/// file then compiled (avoids any shell-escaping of the source).
fn code_command(lang: Language, code: &str) -> Vec<String> {
    match lang {
        Language::Python => vec!["python".into(), "-c".into(), code.into()],
        Language::Bash => vec!["bash".into(), "-c".into(), code.into()],
        Language::Js => vec!["node".into(), "-e".into(), code.into()],
        Language::Rust => {
            let b64 = base64::engine::general_purpose::STANDARD.encode(code);
            let script = format!(
                "echo {b64} | base64 -d > {WORKDIR}/main.rs && \
                 rustc {WORKDIR}/main.rs -o {WORKDIR}/main && {WORKDIR}/main"
            );
            vec!["sh".into(), "-c".into(), script]
        }
    }
}

#[async_trait]
impl SandboxHandle for DockerHandle {
    fn info(&self) -> &SandboxInfo {
        &self.info
    }

    async fn exec(&self, req: ExecRequest) -> Result<ExecResult, SandboxError> {
        self.info.profile.ensure_supports(req.language)?;
        let started_at = Utc::now();
        let timeout = req
            .timeout_secs
            .map(Duration::from_secs)
            .or_else(|| Some(self.info.budget.wall_clock()));

        let mut stderr_acc = String::new();
        // Install dependencies first (best-effort; failure surfaces in stderr).
        if let Some(deps_cmd) = deps_command(req.language, &req.dependencies) {
            let (_o, e, code, _t) = self.run(deps_cmd, &req.env, timeout).await?;
            stderr_acc.push_str(&e);
            if code != 0 {
                let ended_at = Utc::now();
                return Ok(ExecResult {
                    exec_id: ExecId::new(),
                    exit: ExitStatus::from_code(code as i32),
                    stdout: String::new(),
                    stderr: stderr_acc,
                    started_at,
                    ended_at,
                    timed_out: false,
                });
            }
        }

        let (stdout, stderr, code, timed_out) = self
            .run(code_command(req.language, &req.code), &req.env, timeout)
            .await?;
        stderr_acc.push_str(&stderr);
        let ended_at = Utc::now();
        Ok(ExecResult {
            exec_id: ExecId::new(),
            exit: ExitStatus::from_code(code as i32),
            stdout,
            stderr: stderr_acc,
            started_at,
            ended_at,
            timed_out,
        })
    }

    async fn write_file(&self, path: &str, bytes: &[u8]) -> Result<(), SandboxError> {
        let rel = confine(path)?;
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        builder
            .append_data(&mut header, &rel, bytes)
            .map_err(|e| SandboxError::Backend(format!("tar: {e}")))?;
        let data = builder
            .into_inner()
            .map_err(|e| SandboxError::Backend(format!("tar: {e}")))?;
        self.docker
            .upload_to_container(
                &self.container_id,
                Some(UploadToContainerOptions { path: WORKDIR, ..Default::default() }),
                data.into(),
            )
            .await
            .map_err(map_docker)?;
        Ok(())
    }

    async fn read_file(&self, path: &str) -> Result<Vec<u8>, SandboxError> {
        let rel = confine(path)?;
        let full = format!("{WORKDIR}/{rel}");
        let mut stream = self.docker.download_from_container(
            &self.container_id,
            Some(DownloadFromContainerOptions { path: full }),
        );
        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = stream.next().await {
            buf.extend_from_slice(&chunk.map_err(map_docker)?);
        }
        let mut archive = tar::Archive::new(&buf[..]);
        let entries = archive
            .entries()
            .map_err(|e| SandboxError::Backend(format!("tar: {e}")))?;
        for entry in entries {
            let mut e = entry.map_err(|e| SandboxError::Backend(format!("tar: {e}")))?;
            if e.header().entry_type().is_file() {
                let mut out = Vec::new();
                e.read_to_end(&mut out)
                    .map_err(|e| SandboxError::Backend(format!("tar: {e}")))?;
                return Ok(out);
            }
        }
        Err(SandboxError::Guest(format!("file not found: {path}")))
    }

    async fn snapshot(&self) -> Result<SnapshotId, SandboxError> {
        let tag = uuid::Uuid::new_v4().simple().to_string();
        let repo = "atomr-sandbox";
        self.docker
            .commit_container(
                CommitContainerOptions {
                    container: self.container_id.clone(),
                    repo: repo.to_string(),
                    tag: tag.clone(),
                    pause: true,
                    ..Default::default()
                },
                ContainerConfig::<String>::default(),
            )
            .await
            .map_err(map_docker)?;
        Ok(SnapshotId::from(format!("{repo}:{tag}")))
    }

    async fn fork(&self) -> Result<Box<dyn SandboxHandle>, SandboxError> {
        // Commit current state to an image, then boot a fresh container from it.
        let snap = self.snapshot().await?;
        let backend = DockerBackend::with_docker(self.docker.clone(), self.config.clone());
        let container_id = backend.launch_container(snap.as_str()).await?;
        let mut info = self.info.clone();
        info.id = SandboxId::new();
        info.forked_from = Some(snap);
        info.created_at = Utc::now();
        Ok(Box::new(DockerHandle {
            docker: self.docker.clone(),
            config: self.config.clone(),
            container_id,
            info,
        }))
    }

    async fn destroy(&self) -> Result<(), SandboxError> {
        let _ = self
            .docker
            .kill_container(&self.container_id, Some(KillContainerOptions { signal: "SIGKILL" }))
            .await;
        self.docker
            .remove_container(
                &self.container_id,
                Some(RemoveContainerOptions { force: true, ..Default::default() }),
            )
            .await
            .map_err(map_docker)?;
        Ok(())
    }
}
