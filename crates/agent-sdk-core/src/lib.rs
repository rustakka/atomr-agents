//! Uniform contract for the **Claude Agent SDK** harness.
//!
//! This crate wraps Anthropic's programmable Claude Code agent
//! (`claude-agent-sdk` / `@anthropic-ai/claude-agent-sdk`) — the SDK that
//! exposes Claude Code's full harness (slash commands, subagents, hooks,
//! MCP, permission modes, sessions, custom system prompts, built-in
//! Read/Write/Edit/Bash/Grep/WebSearch tools) and bills against your
//! Anthropic API credits by shelling out to the bundled `claude` CLI.
//!
//! It is **distinct** from the server-side "Managed Agents" API.
//!
//! The crate is the contract layer only:
//!
//! * [`AgentSdkConfig`] mirrors the SDK's `ClaudeAgentOptions` and
//!   round-trips to a JSON dict the Python wrapper turns back into
//!   `ClaudeAgentOptions`.
//! * [`AgentSdkMessage`] is the normalized message protocol the backend
//!   yields (the Python wrapper produces these dicts from SDK message
//!   objects).
//! * [`AgentSdkBackend`] / [`AgentSdkSession`] are the pluggable seam.
//!   [`MockBackend`] keeps Rust builds/tests network-free; the real
//!   `PythonAgentSdkBackend` lives in `py-bindings`. A future pure-Rust
//!   backend that spawns the `claude` CLI directly can slot in here too.
//!
//! See the workspace `docs/agent-sdk-harness.md` for the full design.

#![forbid(unsafe_code)]

mod backend;
mod config;
mod error;
mod event;
mod message;
mod mock;
mod request;
mod result;

pub use backend::{AgentSdkBackend, AgentSdkSession, MessageStream};
pub use config::{
    AgentSdkConfig, Effort, HookConfig, HookEvent, HookHandlerRef, McpServerConfig, PermissionMode,
    PermissionPolicy, SettingSource, SubagentDef, SystemPromptConfig, SystemPromptPreset,
};
pub use error::AgentSdkError;
pub use event::{AgentSdkEvent, AgentSdkEventStream, FinishReason};
pub use message::{AgentSdkMessage, ContentBlock};
pub use mock::{MockBackend, MockSession};
pub use request::{AgentRunId, AgentSessionId, QueryRequest, SessionSpec};
pub use result::{ResultSummary, UsageSummary};
