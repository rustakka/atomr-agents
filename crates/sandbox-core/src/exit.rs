//! Process exit status. Kept shape-identical to
//! `coding-cli-isolator::ExitStatus` (copied, not imported, to avoid
//! inverting the core → backend layering), with serde derives added so it
//! rides inside [`ExecResult`](crate::ExecResult).

use serde::{Deserialize, Serialize};

/// Final exit status from a sandboxed process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
