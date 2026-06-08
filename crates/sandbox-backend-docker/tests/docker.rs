//! Integration tests for the Docker backend.
//!
//! These skip gracefully when no Docker daemon is reachable, or when the test
//! image can't be provisioned (e.g. CI without registry egress). Set
//! `ATOMR_SANDBOX_TEST_IMAGE` to a locally-available Python image to run them
//! offline (default `python:3.11-slim`).

use atomr_agents_sandbox_backend_docker::DockerBackend;
use atomr_agents_sandbox_core::{
    CreateSandbox, ExecRequest, Language, SandboxBackend, SandboxBackendSel, SandboxHandle,
    SandboxProfile,
};

fn test_image() -> String {
    std::env::var("ATOMR_SANDBOX_TEST_IMAGE").unwrap_or_else(|_| "python:3.11-slim".to_string())
}

fn python_req() -> CreateSandbox {
    CreateSandbox::new(SandboxProfile::PythonOnly)
        .with_backend(SandboxBackendSel::Docker { image: Some(test_image()) })
}

async fn backend_or_skip() -> Option<DockerBackend> {
    let b = DockerBackend::local().ok()?;
    if b.available().await {
        Some(b)
    } else {
        eprintln!("skipping: no Docker daemon reachable");
        None
    }
}

/// Provision a Python sandbox, or `None` (skip) if the image can't be pulled in
/// this environment.
async fn handle_or_skip(backend: &DockerBackend) -> Option<Box<dyn SandboxHandle>> {
    match backend.create(python_req()).await {
        Ok(h) => Some(h),
        Err(e) => {
            eprintln!("skipping: cannot provision container ({e})");
            None
        }
    }
}

#[tokio::test]
async fn python_exec_files_and_fork() {
    let Some(backend) = backend_or_skip().await else { return };
    let Some(handle) = handle_or_skip(&backend).await else { return };
    assert_eq!(handle.info().backend, "docker");

    // Real code execution.
    let res = handle
        .exec(ExecRequest::new(Language::Python, "print(2 + 2)"))
        .await
        .expect("exec");
    assert!(res.exit.success, "stderr: {}", res.stderr);
    assert_eq!(res.stdout.trim(), "4");

    // File round-trip + visibility from inside the sandbox.
    handle.write_file("data/hello.txt", b"hi from host").await.expect("write");
    assert_eq!(handle.read_file("data/hello.txt").await.expect("read"), b"hi from host");
    let res = handle
        .exec(ExecRequest::new(
            Language::Python,
            "print(open('data/hello.txt').read())",
        ))
        .await
        .expect("exec read");
    assert!(res.stdout.contains("hi from host"), "{} / {}", res.stdout, res.stderr);

    // Fork carries committed filesystem state.
    let child = handle.fork().await.expect("fork");
    assert_ne!(child.info().id, handle.info().id);
    let res = child
        .exec(ExecRequest::new(
            Language::Python,
            "print(open('data/hello.txt').read())",
        ))
        .await
        .expect("child exec");
    assert!(res.stdout.contains("hi from host"));

    child.destroy().await.expect("destroy child");
    handle.destroy().await.expect("destroy");
}

#[tokio::test]
async fn nonzero_exit_is_reported() {
    let Some(backend) = backend_or_skip().await else { return };
    let Some(handle) = handle_or_skip(&backend).await else { return };
    let res = handle
        .exec(ExecRequest::new(Language::Python, "import sys; sys.exit(3)"))
        .await
        .expect("exec");
    assert!(!res.exit.success);
    assert_eq!(res.exit.code, Some(3));
    handle.destroy().await.expect("destroy");
}

#[tokio::test]
async fn path_traversal_is_rejected() {
    let Some(backend) = backend_or_skip().await else { return };
    let Some(handle) = handle_or_skip(&backend).await else { return };
    assert!(handle.write_file("../escape.txt", b"x").await.is_err());
    assert!(handle.write_file("/etc/escape", b"x").await.is_err());
    assert!(handle.read_file("../../etc/passwd").await.is_err());
    handle.destroy().await.expect("destroy");
}

#[tokio::test]
async fn unsupported_language_is_rejected_before_exec() {
    let Some(backend) = backend_or_skip().await else { return };
    let Some(handle) = handle_or_skip(&backend).await else { return };
    assert!(handle
        .exec(ExecRequest::new(Language::Rust, "fn main(){}"))
        .await
        .is_err());
    handle.destroy().await.expect("destroy");
}
