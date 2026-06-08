//! Integration tests for `SandboxHarness` over the mock backend.

use atomr_agents_callable::Callable;
use atomr_agents_core::{CallCtx, IterationBudget, MoneyBudget, TimeBudget, TokenBudget, Value};
use atomr_agents_sandbox_core::{
    CreateSandbox, ExecRequest, Language, SandboxBackendSel, SandboxEvent, SandboxProfile,
};
use atomr_agents_sandbox_harness::{SandboxHarness, SandboxHarnessConfig};
use std::sync::Arc;
use std::time::Duration;

fn call_ctx() -> CallCtx {
    CallCtx {
        agent_id: None,
        tokens: TokenBudget::new(1000),
        time: TimeBudget::new(Duration::from_secs(5)),
        money: MoneyBudget::from_usd(1.0),
        iterations: IterationBudget::new(5),
        trace: vec![],
        extensions: Default::default(),
    }
}

#[tokio::test]
async fn run_once_emits_full_lifecycle_and_returns_output() {
    let h = SandboxHarness::local_default();
    let mut events = h.events();

    let res = h
        .run_exec(
            ExecRequest::new(Language::Python, "print('hi')"),
            None,
            SandboxBackendSel::Auto,
        )
        .await
        .unwrap();
    assert!(res.exit.success);

    // Drain the lifecycle: Created, ExecStarted, ExecEnded, Destroyed.
    let mut kinds = Vec::new();
    while let Ok(Some(ev)) =
        tokio::time::timeout(Duration::from_millis(200), events.recv()).await
    {
        let tag = match ev {
            SandboxEvent::Created { .. } => "created",
            SandboxEvent::ExecStarted { .. } => "exec_started",
            SandboxEvent::ExecEnded { .. } => "exec_ended",
            SandboxEvent::Destroyed { .. } => "destroyed",
            other => panic!("unexpected event: {other:?}"),
        };
        kinds.push(tag);
        if kinds.len() == 4 {
            break;
        }
    }
    assert_eq!(kinds, ["created", "exec_started", "exec_ended", "destroyed"]);
}

#[tokio::test]
async fn persistent_create_exec_fork_destroy() {
    let h = SandboxHarness::local_default();
    let info = h.create(CreateSandbox::new(SandboxProfile::FullStack)).await.unwrap();
    assert_eq!(h.live_count(), 1);

    let handle = h.get(&info.id).unwrap();
    handle.write_file("src/main.rs", b"fn main(){}").await.unwrap();

    let res = h
        .exec(&info.id, ExecRequest::new(Language::Rust, "fn main(){}"))
        .await
        .unwrap();
    assert!(res.exit.success);

    // Fork registers a second sandbox carrying the parent's filesystem.
    let child = h.fork(&info.id).await.unwrap();
    assert_eq!(h.live_count(), 2);
    let child_handle = h.get(&child.id).unwrap();
    assert_eq!(child_handle.read_file("src/main.rs").await.unwrap(), b"fn main(){}");

    h.destroy(&info.id).await.unwrap();
    h.destroy(&child.id).await.unwrap();
    assert_eq!(h.live_count(), 0);
}

#[tokio::test]
async fn concurrency_quota_is_enforced() {
    let config = SandboxHarnessConfig { max_concurrent_sandboxes: 1, ..Default::default() };
    let h = SandboxHarness::new(
        Arc::new(atomr_agents_sandbox_core::MockBackend::new()),
        Arc::new(atomr_agents_sandbox_harness::BestFitScheduler),
        config,
    );
    h.create(CreateSandbox::new(SandboxProfile::PythonOnly)).await.unwrap();
    let err = h.create(CreateSandbox::new(SandboxProfile::PythonOnly)).await.unwrap_err();
    assert!(err.to_string().contains("max concurrent"));
}

#[tokio::test]
async fn unknown_id_is_not_found() {
    let h = SandboxHarness::local_default();
    let bogus = atomr_agents_sandbox_core::SandboxId::new();
    let err = h.exec(&bogus, ExecRequest::new(Language::Bash, "echo hi")).await.unwrap_err();
    assert!(err.to_string().contains("not found"));
}

#[tokio::test]
async fn callable_runs_an_exec_from_json() {
    let h = SandboxHarness::local_default();
    let input: Value = serde_json::json!({
        "language": "python",
        "code": "print(2+2)",
        "profile": "python_only"
    });
    let out = h.call(input, call_ctx()).await.unwrap();
    // Unified flat tool shape: { exec_id, exit_code, success, stdout, stderr, timed_out }.
    assert_eq!(out.get("success").and_then(|v| v.as_bool()), Some(true));
    assert!(out.get("exit_code").is_some());
    assert!(out.get("stdout").unwrap().as_str().unwrap().contains("[mock:Python]"));
}

#[tokio::test]
async fn callable_error_path_has_sandbox_prefix() {
    let h = SandboxHarness::local_default();
    // python_only profile + rust language → run_exec fails the supports guard.
    let input: Value = serde_json::json!({
        "language": "rust",
        "code": "fn main(){}",
        "profile": "python_only"
    });
    let err = h.call(input, call_ctx()).await.unwrap_err();
    match err {
        atomr_agents_core::AgentError::Tool(m) => {
            assert!(m.starts_with("sandbox: "), "got: {m}");
            assert!(m.contains("cannot run language"));
        }
        other => panic!("expected Tool err, got {other:?}"),
    }
    // Malformed input (missing required `code`) is a serde error, still Err.
    let bad = h
        .call(serde_json::json!({ "language": "python" }), call_ctx())
        .await;
    assert!(bad.is_err());
}

#[tokio::test]
async fn run_exec_profile_mismatch_errors_before_any_event() {
    let h = SandboxHarness::local_default();
    let mut events = h.events();
    let err = h
        .run_exec(
            ExecRequest::new(Language::Rust, "fn main(){}"),
            Some(SandboxProfile::PythonOnly),
            SandboxBackendSel::Auto,
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("cannot run language"));
    // No sandbox was created, so no event is emitted.
    let got = tokio::time::timeout(Duration::from_millis(100), events.recv()).await;
    assert!(got.is_err(), "expected no event, got {got:?}");
}

#[tokio::test]
async fn exec_failure_emits_exec_error_not_exec_ended() {
    let h = SandboxHarness::local_default();
    let info = h.create(CreateSandbox::new(SandboxProfile::PythonOnly)).await.unwrap();
    let mut events = h.events();
    let err = h
        .exec(&info.id, ExecRequest::new(Language::Rust, "fn main(){}"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("cannot run language"));

    let mut saw_error = false;
    while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(150), events.recv()).await {
        match ev {
            SandboxEvent::ExecStarted { .. } => {}
            SandboxEvent::ExecError { id, error } => {
                assert_eq!(id, info.id);
                assert!(error.contains("cannot run language"));
                saw_error = true;
                break;
            }
            SandboxEvent::ExecEnded { .. } => panic!("exec failed, should not emit ExecEnded"),
            _ => {}
        }
    }
    assert!(saw_error, "expected an ExecError event");
}

#[tokio::test]
async fn fork_emits_forked_event_with_lineage() {
    let h = SandboxHarness::local_default();
    let parent = h.create(CreateSandbox::new(SandboxProfile::FullStack)).await.unwrap();
    let mut events = h.events();
    let child = h.fork(&parent.id).await.unwrap();

    let mut saw = false;
    while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(150), events.recv()).await {
        if let SandboxEvent::Forked { parent: p, child: c, snapshot } = ev {
            assert_eq!(p, parent.id);
            assert_eq!(c, child.id);
            assert!(!snapshot.as_str().is_empty());
            saw = true;
            break;
        }
    }
    assert!(saw, "expected a Forked event");
}

#[tokio::test]
async fn snapshot_succeeds_and_unknown_id_is_not_found() {
    let h = SandboxHarness::local_default();
    let info = h.create(CreateSandbox::new(SandboxProfile::PythonOnly)).await.unwrap();
    let snap = h.snapshot(&info.id).await.unwrap();
    assert!(snap.as_str().starts_with("snap-"));

    let bogus = atomr_agents_sandbox_core::SandboxId::new();
    assert!(h.snapshot(&bogus).await.unwrap_err().to_string().contains("not found"));
}

#[tokio::test]
async fn rust_floor_is_normalized_at_the_boundary() {
    // Even with an undersized explicit budget, a Rust profile is provisioned at
    // the 2 GB / 2 vCPU floor (the harness normalizes before dispatch).
    let h = SandboxHarness::local_default();
    let req = CreateSandbox::new(SandboxProfile::RustOnly).with_budget(
        atomr_agents_sandbox_core::ResourceBudget {
            vcpus: 1,
            mem_mib: 128,
            disk_mib: 1024,
            wall_clock_secs: 10,
        },
    );
    let info = h.create(req).await.unwrap();
    assert!(info.budget.vcpus >= 2);
    assert!(info.budget.mem_mib >= 2048);
}

#[tokio::test]
async fn explicit_unavailable_backend_is_rejected() {
    let h = SandboxHarness::local_default(); // mock backend
    // Asking for Firecracker on a mock harness must fail, not silently downgrade.
    let err = h
        .run_exec(
            ExecRequest::new(Language::Python, "print(1)"),
            None,
            SandboxBackendSel::Firecracker,
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not available"));

    let err = h
        .create(CreateSandbox::new(SandboxProfile::PythonOnly).with_backend(
            SandboxBackendSel::Remote { endpoint: "grpc://x".into() },
        ))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not available"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_creates_respect_quota() {
    // Race many concurrent create() calls against a cap of 3; the atomic
    // reservation must keep the registry from ever exceeding the cap.
    let config = SandboxHarnessConfig { max_concurrent_sandboxes: 3, ..Default::default() };
    let h = Arc::new(SandboxHarness::new(
        Arc::new(atomr_agents_sandbox_core::MockBackend::new()),
        Arc::new(atomr_agents_sandbox_harness::BestFitScheduler),
        config,
    ));
    let mut handles = Vec::new();
    for _ in 0..16 {
        let h = h.clone();
        handles.push(tokio::spawn(async move {
            h.create(CreateSandbox::new(SandboxProfile::PythonOnly)).await.is_ok()
        }));
    }
    let mut ok = 0;
    for jh in handles {
        if jh.await.unwrap() {
            ok += 1;
        }
    }
    assert_eq!(ok, 3, "exactly cap-many creates should succeed");
    assert_eq!(h.live_count(), 3);
}
