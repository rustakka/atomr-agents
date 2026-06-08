//! Long-lived interactive sessions (the `ClaudeSDKClient` surface).
//!
//! One [`InteractiveAgentSession`] wraps a backend session. `send()` queues
//! a prompt and spawns a pump task that drains the response stream,
//! projecting each message onto the broadcast event stream. Mirrors the
//! `SessionRegistry` shape from `coding-cli-harness`.

use std::sync::Arc;

use futures::StreamExt;
use parking_lot::RwLock;
use tokio::sync::broadcast;

use atomr_agents_agent::SpendLedger;
use atomr_agents_agent_sdk_core::{
    AgentSdkEvent, AgentSdkEventStream, AgentSdkSession, AgentSessionId,
};
use atomr_agents_core::HarnessId;
use atomr_agents_observability::EventBus;

use crate::bridge;
use crate::error::Result;

pub struct InteractiveAgentSession {
    inner: Arc<dyn AgentSdkSession>,
    pub id: AgentSessionId,
    event_tx: broadcast::Sender<AgentSdkEvent>,
    bus: EventBus,
    ledger: SpendLedger,
    harness_id: HarnessId,
}

impl InteractiveAgentSession {
    pub(crate) fn new(
        inner: Box<dyn AgentSdkSession>,
        event_tx: broadcast::Sender<AgentSdkEvent>,
        bus: EventBus,
        ledger: SpendLedger,
        harness_id: HarnessId,
    ) -> Self {
        let inner: Arc<dyn AgentSdkSession> = Arc::from(inner);
        let id = inner.session_id().clone();
        Self {
            inner,
            id,
            event_tx,
            bus,
            ledger,
            harness_id,
        }
    }

    /// Queue a prompt and start streaming the agent's response onto the
    /// event channel.
    pub async fn send(&self, prompt: String) -> Result<()> {
        self.inner.send(prompt).await?;
        let inner = self.inner.clone();
        let event_tx = self.event_tx.clone();
        let bus = self.bus.clone();
        let ledger = self.ledger.clone();
        let hid = self.harness_id.clone();
        tokio::spawn(async move {
            if let Ok(mut stream) = inner.receive().await {
                while let Some(item) = stream.next().await {
                    match item {
                        Ok(msg) => {
                            let _ = bridge::project(&msg, &hid, &event_tx, &bus, &ledger);
                        }
                        Err(_) => break,
                    }
                }
            }
        });
        Ok(())
    }

    /// Subscribe to this session's event stream.
    pub fn subscribe(&self) -> AgentSdkEventStream {
        AgentSdkEventStream::new(self.event_tx.subscribe())
    }

    /// Raw broadcast receiver (used by the actor `Subscribe` message).
    pub fn subscribe_receiver(&self) -> broadcast::Receiver<AgentSdkEvent> {
        self.event_tx.subscribe()
    }

    pub async fn interrupt(&self) -> Result<()> {
        self.inner.interrupt().await?;
        Ok(())
    }

    pub async fn set_permission_mode(&self, mode: String) -> Result<()> {
        self.inner.set_permission_mode(mode).await?;
        Ok(())
    }

    pub async fn set_model(&self, model: String) -> Result<()> {
        self.inner.set_model(model).await?;
        Ok(())
    }

    pub async fn close(&self) -> Result<()> {
        self.inner.close().await?;
        Ok(())
    }
}

/// Concurrent registry of active sessions — shared by the harness and the
/// web companion.
#[derive(Default, Clone)]
pub struct SessionRegistry {
    inner: Arc<RwLock<Vec<Arc<InteractiveAgentSession>>>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, h: Arc<InteractiveAgentSession>) {
        self.inner.write().push(h);
    }

    pub fn get(&self, id: &AgentSessionId) -> Option<Arc<InteractiveAgentSession>> {
        self.inner.read().iter().find(|s| &s.id == id).cloned()
    }

    pub fn list(&self) -> Vec<Arc<InteractiveAgentSession>> {
        self.inner.read().clone()
    }

    pub fn remove(&self, id: &AgentSessionId) {
        self.inner.write().retain(|s| &s.id != id);
    }

    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.read().is_empty()
    }
}
