//! `sandbox-guest-agent` — the in-VM daemon that runs as **PID 1** inside a
//! Firecracker microVM guest.
//!
//! It serves the [`atomr_agents_sandbox_proto`] command set. The production
//! transport is `AF_VSOCK`; this binary speaks the protocol over **stdin /
//! stdout** so the host (the Linux Firecracker backend) bridges the guest's
//! stdio onto the vsock device — keeping the binary free of platform-specific
//! socket code and fully testable. On Linux it also performs best-effort PID-1
//! init (mounting `/proc`, `/sys`, `/tmp`).

use atomr_agents_sandbox_guest_agent::{serve_rw, GuestConfig};

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    let code = rt.block_on(async {
        #[cfg(target_os = "linux")]
        best_effort_init().await;

        let config = GuestConfig::default();
        let _ = tokio::fs::create_dir_all(&config.root).await;

        match serve_rw(tokio::io::stdin(), tokio::io::stdout(), &config).await {
            Ok(_) => 0,
            Err(e) => {
                eprintln!("guest-agent: fatal: {e}");
                1
            }
        }
    });

    std::process::exit(code);
}

/// Mount the pseudo-filesystems the toolchains expect. Best-effort: failures
/// are non-fatal (the kernel cmdline may already mount them).
#[cfg(target_os = "linux")]
async fn best_effort_init() {
    for (src, target, fstype) in [
        ("proc", "/proc", "proc"),
        ("sysfs", "/sys", "sysfs"),
        ("tmpfs", "/tmp", "tmpfs"),
    ] {
        let _ = tokio::fs::create_dir_all(target).await;
        let _ = tokio::process::Command::new("mount")
            .args(["-t", fstype, src, target])
            .status()
            .await;
    }
}
