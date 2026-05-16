use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::Callable;
use atomr_agents_core::{CallCtx, Message, MessageRole, Value};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::content::{InboundMessage, MessageContent};
use crate::ids::{ChannelId, PeerId, ThreadId};
use crate::target::ThreadTarget;

/// Lightweight policy controlling thread behavior.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ThreadPolicy {
    /// Maximum number of `Message`s kept in `history`. Older messages
    /// are evicted from the front. `0` means unbounded.
    #[serde(default = "ThreadPolicy::default_history_cap")]
    pub history_cap: usize,
    /// If `true`, an unknown peer's first inbound auto-opens a thread
    /// using the channel's default target.
    #[serde(default)]
    pub auto_open: bool,
}

impl ThreadPolicy {
    fn default_history_cap() -> usize {
        200
    }
}

impl Default for ThreadPolicy {
    fn default() -> Self {
        Self {
            history_cap: Self::default_history_cap(),
            auto_open: false,
        }
    }
}

/// One conversation between a peer and a bound target on a channel.
///
/// Stored in the [`ChannelStore`](crate::ChannelStore) by id; the
/// orchestrator holds an `Arc<RwLock<Thread>>` through [`ThreadRef`].
#[derive(Clone)]
pub struct Thread {
    pub id: ThreadId,
    pub channel: ChannelId,
    pub peer: PeerId,
    pub target: ThreadTarget,
    pub history: Vec<Message>,
    pub policy: ThreadPolicy,
}

impl Thread {
    pub fn new(channel: ChannelId, peer: PeerId, target: ThreadTarget) -> Self {
        let id = ThreadId::for_peer(&channel, &peer);
        Self {
            id,
            channel,
            peer,
            target,
            history: Vec::new(),
            policy: ThreadPolicy::default(),
        }
    }

    pub fn push_user(&mut self, content: &MessageContent) {
        self.push(MessageRole::User, content.as_text());
    }

    pub fn push_assistant(&mut self, content: &MessageContent) {
        self.push(MessageRole::Assistant, content.as_text());
    }

    pub fn push(&mut self, role: MessageRole, text: String) {
        self.history.push(Message { role, content: text });
        if self.policy.history_cap > 0 && self.history.len() > self.policy.history_cap {
            let excess = self.history.len() - self.policy.history_cap;
            self.history.drain(0..excess);
        }
    }
}

/// Public, shared handle on a [`Thread`]. Implements [`Callable`] so
/// a thread can be embedded as a workflow step or team child.
#[derive(Clone)]
pub struct ThreadRef {
    inner: Arc<RwLock<Thread>>,
}

impl ThreadRef {
    pub fn from_arc(inner: Arc<RwLock<Thread>>) -> Self {
        Self { inner }
    }

    pub fn new(thread: Thread) -> Self {
        Self {
            inner: Arc::new(RwLock::new(thread)),
        }
    }

    pub fn read(&self) -> parking_lot::RwLockReadGuard<'_, Thread> {
        self.inner.read()
    }

    pub fn write(&self) -> parking_lot::RwLockWriteGuard<'_, Thread> {
        self.inner.write()
    }

    pub fn id(&self) -> ThreadId {
        self.inner.read().id.clone()
    }

    pub fn snapshot(&self) -> Thread {
        self.inner.read().clone()
    }

    /// Synthesize an inbound from a free-text string for ad-hoc callers
    /// (mainly the `Callable` impl). Provider-driven inbound goes
    /// through the orchestrator instead.
    pub fn synthetic_inbound(&self, text: String) -> InboundMessage {
        let t = self.inner.read();
        InboundMessage {
            channel_id: t.channel.clone(),
            thread_id: t.id.clone(),
            peer: t.peer.clone(),
            provider_msg_id: format!("synthetic-{}", uuid::Uuid::new_v4()),
            content: MessageContent::text(text),
            received_at: chrono::Utc::now(),
            raw: serde_json::Value::Null,
        }
    }
}

/// `Callable` adapter — lets a thread be embedded anywhere a
/// `Callable` is expected.
///
/// For `ThreadTarget::Callable` targets, we forward `{"user": text, …}`
/// envelopes that `AgentRef::call` already understands. For
/// `ThreadTarget::Harness` targets we call the wrapped callable (which
/// is `HarnessRef`); see [`crate::ThreadTarget`].
#[async_trait]
impl Callable for ThreadRef {
    async fn call(&self, input: Value, ctx: CallCtx) -> atomr_agents_core::Result<Value> {
        let text = extract_user_text(&input);
        let (target, channel, peer, thread_id, history) = {
            let g = self.inner.read();
            (
                g.target.clone(),
                g.channel.clone(),
                g.peer.clone(),
                g.id.clone(),
                g.history.clone(),
            )
        };
        let envelope = serde_json::json!({
            "user": text,
            "thread": { "id": thread_id.as_str(), "history_len": history.len() },
            "channel": { "id": channel.as_str(), "peer": peer.as_str() },
        });
        match target {
            ThreadTarget::Callable(handle) => handle.call(envelope, ctx).await,
            ThreadTarget::Harness { callable, .. } => callable.call(envelope, ctx).await,
        }
    }

    fn label(&self) -> &str {
        "thread"
    }
}

fn extract_user_text(input: &Value) -> String {
    match input {
        Value::String(s) => s.clone(),
        Value::Object(m) => m
            .get("user")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        other => other.to_string(),
    }
}
