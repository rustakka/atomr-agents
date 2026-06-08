//! Public-surface tests for `sandbox-core`: budget floor enforcement, profile
//! language mapping, JSON round-trips, and the `MockBackend` lifecycle.

use atomr_agents_sandbox_core::{
    CreateSandbox, ExecRequest, ExecResult, Language, MockBackend, ResourceBudget, SandboxBackend,
    SandboxBackendSel, SandboxProfile,
};

#[test]
fn rust_profiles_enforce_2gb_2vcpu_floor() {
    for p in [SandboxProfile::RustOnly, SandboxProfile::FullStack] {
        let b = ResourceBudget::for_profile(p);
        assert!(b.vcpus >= ResourceBudget::RUST_MIN_VCPUS, "{p:?} vcpus");
        assert!(b.mem_mib >= ResourceBudget::RUST_MIN_MEM_MIB, "{p:?} mem");
    }
}

#[test]
fn interpreted_profiles_use_light_defaults() {
    let b = ResourceBudget::for_profile(SandboxProfile::PythonOnly);
    assert_eq!(b.vcpus, 1);
    assert_eq!(b.mem_mib, 512);
}

#[test]
fn rust_floor_is_idempotent_and_only_raises() {
    // A caller trying to under-provision a Rust profile is lifted to the floor.
    let undersized = ResourceBudget { vcpus: 1, mem_mib: 256, disk_mib: 1024, wall_clock_secs: 30 };
    let lifted = undersized.enforce_rust_floor();
    assert_eq!(lifted.vcpus, 2);
    assert_eq!(lifted.mem_mib, 2048);
    // Idempotent.
    assert_eq!(lifted.enforce_rust_floor(), lifted);
    // A caller over-provisioning keeps their larger values.
    let big = ResourceBudget { vcpus: 8, mem_mib: 8192, disk_mib: 4096, wall_clock_secs: 30 };
    assert_eq!(big.enforce_rust_floor(), big);
}

#[test]
fn create_sandbox_effective_budget_reapplies_rust_floor() {
    let req = CreateSandbox::new(SandboxProfile::RustOnly).with_budget(ResourceBudget {
        vcpus: 1,
        mem_mib: 128,
        disk_mib: 1024,
        wall_clock_secs: 10,
    });
    let eff = req.effective_budget();
    assert_eq!(eff.vcpus, 2);
    assert_eq!(eff.mem_mib, 2048);
}

#[test]
fn profile_language_support_table() {
    assert!(SandboxProfile::PythonOnly.supports(Language::Python));
    assert!(SandboxProfile::PythonOnly.supports(Language::Bash)); // shell everywhere
    assert!(!SandboxProfile::PythonOnly.supports(Language::Rust));
    assert!(SandboxProfile::NpmOnly.supports(Language::Js));
    assert!(!SandboxProfile::NpmOnly.supports(Language::Python));
    assert!(SandboxProfile::PythonAndNpm.supports(Language::Python));
    assert!(SandboxProfile::PythonAndNpm.supports(Language::Js));
    assert!(!SandboxProfile::PythonAndNpm.supports(Language::Rust));
    for lang in [Language::Python, Language::Bash, Language::Js, Language::Rust] {
        assert!(SandboxProfile::FullStack.supports(lang));
    }
}

#[test]
fn profile_serde_is_snake_case() {
    let j = serde_json::to_string(&SandboxProfile::PythonAndNpm).unwrap();
    assert_eq!(j, "\"python_and_npm\"");
    let back: SandboxProfile = serde_json::from_str("\"full_stack\"").unwrap();
    assert_eq!(back, SandboxProfile::FullStack);
}

#[test]
fn backend_sel_is_internally_tagged() {
    let j = serde_json::to_value(SandboxBackendSel::default()).unwrap();
    assert_eq!(j, serde_json::json!({ "kind": "auto" }));
    let remote: SandboxBackendSel =
        serde_json::from_value(serde_json::json!({ "kind": "remote", "endpoint": "grpc://x" }))
            .unwrap();
    assert_eq!(remote, SandboxBackendSel::Remote { endpoint: "grpc://x".into() });
}

#[test]
fn create_sandbox_and_exec_request_round_trip() {
    let req = CreateSandbox::new(SandboxProfile::FullStack);
    let j = serde_json::to_value(&req).unwrap();
    let back: CreateSandbox = serde_json::from_value(j).unwrap();
    assert_eq!(back, req);

    let exec = ExecRequest::new(Language::Python, "print(2+2)");
    let j = serde_json::to_value(&exec).unwrap();
    let back: ExecRequest = serde_json::from_value(j).unwrap();
    assert_eq!(back, exec);
}

#[tokio::test]
async fn mock_backend_exec_round_trip() {
    let backend = MockBackend::new();
    assert!(backend.available().await);
    let handle = backend
        .create(CreateSandbox::new(SandboxProfile::PythonOnly))
        .await
        .unwrap();
    assert_eq!(handle.info().backend, "mock");
    assert_eq!(handle.info().boot_ms, 0);

    let res: ExecResult = handle
        .exec(ExecRequest::new(Language::Python, "print('hi')"))
        .await
        .unwrap();
    assert!(res.exit.success);
    assert!(res.stdout.contains("[mock:Python]"));
}

#[tokio::test]
async fn mock_backend_rejects_unsupported_language() {
    let backend = MockBackend::new();
    let handle = backend
        .create(CreateSandbox::new(SandboxProfile::PythonOnly))
        .await
        .unwrap();
    let err = handle
        .exec(ExecRequest::new(Language::Rust, "fn main() {}"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("cannot run language"));
}

#[tokio::test]
async fn mock_backend_files_and_fork_carry_state() {
    let backend = MockBackend::new();
    let handle = backend
        .create(CreateSandbox::new(SandboxProfile::FullStack))
        .await
        .unwrap();
    handle.write_file("src/main.rs", b"fn main(){}").await.unwrap();
    assert_eq!(handle.read_file("src/main.rs").await.unwrap(), b"fn main(){}");

    let child = handle.fork().await.unwrap();
    assert_ne!(child.info().id, handle.info().id);
    assert!(child.info().forked_from.is_some());
    // Forked sandbox inherits the parent's filesystem snapshot.
    assert_eq!(child.read_file("src/main.rs").await.unwrap(), b"fn main(){}");

    // Reading a missing file is an error, not a panic.
    assert!(handle.read_file("nope.txt").await.is_err());

    handle.destroy().await.unwrap();
}

#[test]
fn ensure_supports_yields_canonical_error() {
    assert!(SandboxProfile::PythonOnly.ensure_supports(Language::Python).is_ok());
    let err = SandboxProfile::PythonOnly
        .ensure_supports(Language::Rust)
        .unwrap_err();
    assert!(err.to_string().contains("cannot run language"));
}

#[tokio::test]
async fn exec_result_to_tool_json_is_flat() {
    let backend = MockBackend::new();
    let handle = backend
        .create(CreateSandbox::new(SandboxProfile::PythonOnly))
        .await
        .unwrap();
    let res = handle
        .exec(ExecRequest::new(Language::Python, "print(1)"))
        .await
        .unwrap();
    let j = res.to_tool_json();
    for key in ["exec_id", "exit_code", "success", "stdout", "stderr", "timed_out"] {
        assert!(j.get(key).is_some(), "missing {key}");
    }
    // Flat: no nested `exit` object.
    assert!(j.get("exit").is_none());
    assert_eq!(j.get("success").and_then(|v| v.as_bool()), Some(true));
}

#[tokio::test]
async fn result_and_info_serialize_with_expected_shape() {
    let backend = MockBackend::new();
    let handle = backend
        .create(CreateSandbox::new(SandboxProfile::PythonOnly))
        .await
        .unwrap();

    // SandboxInfo: forked_from is None here, so the key is omitted.
    let info_json = serde_json::to_value(handle.info()).unwrap();
    for key in ["id", "profile", "budget", "backend", "boot_ms", "created_at"] {
        assert!(info_json.get(key).is_some(), "missing {key}");
    }
    assert!(info_json.get("forked_from").is_none(), "None forked_from should be skipped");

    // ExecResult: full internal shape (nested exit + timestamps).
    let res = handle
        .exec(ExecRequest::new(Language::Python, "print(1)"))
        .await
        .unwrap();
    let rj = serde_json::to_value(&res).unwrap();
    for key in ["exec_id", "exit", "stdout", "stderr", "started_at", "ended_at", "timed_out"] {
        assert!(rj.get(key).is_some(), "missing {key}");
    }
    // And it round-trips back.
    let back: ExecResult = serde_json::from_value(rj).unwrap();
    assert_eq!(back, res);
}

#[tokio::test]
async fn mock_snapshot_returns_prefixed_id() {
    let backend = MockBackend::new();
    let handle = backend
        .create(CreateSandbox::new(SandboxProfile::PythonOnly))
        .await
        .unwrap();
    let snap = handle.snapshot().await.unwrap();
    assert!(snap.as_str().starts_with("snap-"));
}

#[tokio::test]
async fn exec_streaming_default_impl_streams_then_resolves() {
    let backend = MockBackend::new();
    let handle = backend
        .create(CreateSandbox::new(SandboxProfile::PythonOnly))
        .await
        .unwrap();
    let mut stream = handle
        .exec_streaming(ExecRequest::new(Language::Python, "print(1)"))
        .await
        .unwrap();

    let mut out = Vec::new();
    while let Some(chunk) = stream.stdout_rx.recv().await {
        out.extend_from_slice(&chunk);
    }
    let res = stream.result.await.unwrap();
    assert!(res.exit.success);
    assert!(String::from_utf8(out).unwrap().contains("[mock:Python]"));
}
