//! The pluggable backend abstraction.
//!
//! [`AgentSdkBackend`] is the seam between the harness and whatever actually
//! drives the Claude Agent SDK. [`MockBackend`](crate::MockBackend) keeps
//! Rust tests network-free; `PythonAgentSdkBackend` (in `py-bindings`)
//! drives the real `claude-agent-sdk`. A future pure-Rust backend that
//! spawns the `claude` CLI directly can implement the same traits.

use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;

use crate::error::AgentSdkError;
use crate::message::AgentSdkMessage;
use crate::request::{AgentSessionId, QueryRequest, SessionSpec};

/// A stream of normalized messages. `Result<_, _>` items let a backend
/// surface a mid-stream error (e.g. a Python exception) as a stream error
/// rather than a silent truncation.
pub type MessageStream =
    Pin<Box<dyn Stream<Item = Result<AgentSdkMessage, AgentSdkError>> + Send>>;

/// Drives the Claude Agent SDK — one-shot queries and stateful sessions.
#[async_trait]
pub trait AgentSdkBackend: Send + Sync {
    /// Stable identifier used in logs (`mock`, `python`, `claude-cli`).
    fn name(&self) -> &str;

    /// Whether this backend is usable on the current host (SDK installed,
    /// `claude` CLI present, credentials configured…).
    async fn available(&self) -> bool;

    /// Run a one-shot query (the SDK `query()` path), streaming normalized
    /// messages to a terminal [`Result`](AgentSdkMessage::Result).
    async fn query(&self, req: QueryRequest) -> Result<MessageStream, AgentSdkError>;

    /// Open a stateful interactive session (the SDK `ClaudeSDKClient` path).
    async fn create_session(
        &self,
        spec: SessionSpec,
    ) -> Result<Box<dyn AgentSdkSession>, AgentSdkError>;
}

/// A live interactive session. Maps 1:1 onto `ClaudeSDKClient`.
#[async_trait]
pub trait AgentSdkSession: Send + Sync {
    /// The opaque session handle (registry / actor key).
    fn session_id(&self) -> &AgentSessionId;

    /// Send a user prompt (`ClaudeSDKClient.query()`).
    async fn send(&self, prompt: String) -> Result<(), AgentSdkError>;

    /// Stream the agent's response for the current turn
    /// (`receive_response()`), ending at a terminal result.
    async fn receive(&self) -> Result<MessageStream, AgentSdkError>;

    /// Interrupt the in-flight turn.
    async fn interrupt(&self) -> Result<(), AgentSdkError>;

    /// Change permission mode mid-conversation.
    async fn set_permission_mode(&self, mode: String) -> Result<(), AgentSdkError>;

    /// Change model mid-conversation.
    async fn set_model(&self, model: String) -> Result<(), AgentSdkError>;

    /// Tear down the session and its `claude` subprocess.
    async fn close(&self) -> Result<(), AgentSdkError>;
}
