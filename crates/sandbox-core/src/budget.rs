//! Resource budgets (vCPU / RAM / disk / wall-clock) for a sandbox.
//!
//! The security-relevant invariant lives here: any profile carrying a Rust
//! toolchain is raised to a 2 GB / 2 vCPU floor that callers can exceed but
//! never undercut (see [`ResourceBudget::enforce_rust_floor`]).

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::profile::SandboxProfile;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceBudget {
    pub vcpus: u8,
    pub mem_mib: u32,
    pub disk_mib: u32,
    /// Hard wall-clock cap for a single exec, in seconds.
    pub wall_clock_secs: u64,
}

impl ResourceBudget {
    /// Minimum vCPUs enforced for any Rust-bearing profile (rustc + LLVM are
    /// memory/CPU heavy — see PRD §4.2).
    pub const RUST_MIN_VCPUS: u8 = 2;
    /// Minimum RAM (MiB) enforced for any Rust-bearing profile.
    pub const RUST_MIN_MEM_MIB: u32 = 2048;

    pub const DEFAULT_VCPUS: u8 = 1;
    pub const DEFAULT_MEM_MIB: u32 = 512;
    pub const DEFAULT_DISK_MIB: u32 = 2048;
    pub const DEFAULT_WALL_CLOCK_SECS: u64 = 60;

    /// Default budget for a profile, applying the Rust floor when relevant.
    pub fn for_profile(p: SandboxProfile) -> Self {
        let base = Self {
            vcpus: Self::DEFAULT_VCPUS,
            mem_mib: Self::DEFAULT_MEM_MIB,
            disk_mib: Self::DEFAULT_DISK_MIB,
            wall_clock_secs: Self::DEFAULT_WALL_CLOCK_SECS,
        };
        if p.is_rust() {
            base.enforce_rust_floor()
        } else {
            base
        }
    }

    /// Raise vCPUs/RAM to the Rust minimum. Idempotent, and only ever raises —
    /// a caller-supplied override below the floor is silently lifted, never
    /// honored. Call this whenever a Rust profile is provisioned.
    pub fn enforce_rust_floor(mut self) -> Self {
        self.vcpus = self.vcpus.max(Self::RUST_MIN_VCPUS);
        self.mem_mib = self.mem_mib.max(Self::RUST_MIN_MEM_MIB);
        self
    }

    pub fn wall_clock(&self) -> Duration {
        Duration::from_secs(self.wall_clock_secs)
    }
}

impl Default for ResourceBudget {
    fn default() -> Self {
        Self::for_profile(SandboxProfile::PythonOnly)
    }
}
