//! The guest command set. Defined here (rather than reusing
//! `sandbox-core::Language`) so the guest agent links against this crate alone
//! and stays a lean static binary.

use serde::{Deserialize, Serialize};

/// Language the guest can execute. Mirrors `sandbox_core::Language` on the wire
/// (snake_case), kept separate to keep the guest binary's dependency graph tiny.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Python,
    Bash,
    Js,
    Rust,
}

/// Host → guest request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GuestRequest {
    /// Liveness check; the guest replies [`GuestResponse::Pong`].
    Ping,
    /// Run code. The guest streams `ExecStdout`/`ExecStderr` then `ExecDone`.
    Exec {
        exec_id: String,
        language: Language,
        code: String,
        dependencies: Vec<String>,
        stdin: Option<String>,
        env: Vec<(String, String)>,
        timeout_ms: Option<u64>,
    },
    WriteFile {
        path: String,
        bytes: Vec<u8>,
    },
    ReadFile {
        path: String,
    },
    /// Ask the guest (PID 1) to shut the VM down cleanly.
    Shutdown,
}

/// Guest → host response. Exec produces a stream of `ExecStdout`/`ExecStderr`
/// frames terminated by exactly one `ExecDone`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GuestResponse {
    Pong,
    ExecStdout { exec_id: String, chunk: Vec<u8> },
    ExecStderr { exec_id: String, chunk: Vec<u8> },
    ExecDone { exec_id: String, exit_code: i32, timed_out: bool },
    FileWritten,
    FileRead { bytes: Vec<u8> },
    Error { message: String },
}
