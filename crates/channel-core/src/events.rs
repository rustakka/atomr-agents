//! Channel domain events fanned out over a `tokio::broadcast` channel.
//!
//! Same shape as [`atomr_agents_meetings_harness::MeetingsHarnessEvent`]:
//! an internally-tagged enum that serializes cleanly to JSON for any
//! observer (the `-web` companion bridges it to a WebSocket).

use serde::Serialize;
use tokio::sync::broadcast;

use crate::ids::{ChannelId, PeerId, ThreadId};
use crate::spec::ProviderKind;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChannelEvent {
    /// A provider task started successfully.
    ProviderConnected {
        provider: ProviderKind,
        channel_id: ChannelId,
    },
    /// A provider task ended.
    ProviderDisconnected {
        provider: ProviderKind,
        channel_id: ChannelId,
        reason: String,
    },
    /// A new thread was opened — explicitly or auto-opened on first
    /// inbound from this peer.
    ThreadOpened {
        thread_id: ThreadId,
        channel_id: ChannelId,
        peer: PeerId,
    },
    /// A thread was closed.
    ThreadClosed {
        thread_id: ThreadId,
        reason: String,
    },
    /// An inbound message landed and was accepted (deduplicated).
    MessageReceived {
        thread_id: ThreadId,
        message_id: String,
        peer: PeerId,
        summary: String,
    },
    /// A duplicate inbound was dropped.
    MessageDuplicate {
        thread_id: ThreadId,
        provider_msg_id: String,
    },
    /// The orchestrator started invoking the bound target for an inbound message.
    TurnStarted {
        thread_id: ThreadId,
        message_id: String,
    },
    /// The bound target returned. `output_summary` is short.
    TurnCompleted {
        thread_id: ThreadId,
        message_id: String,
        output_summary: String,
    },
    /// An outbound message was sent and acked by the provider.
    MessageSent {
        thread_id: ThreadId,
        message_id: String,
        provider_msg_id: String,
    },
    /// A non-fatal error in the inbound or outbound path.
    Error {
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<ThreadId>,
        #[serde(skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
        reason: String,
    },
}

/// Subscriber handle for [`ChannelEvent`]s.
pub struct ChannelEventStream {
    rx: broadcast::Receiver<ChannelEvent>,
}

impl ChannelEventStream {
    pub fn new(rx: broadcast::Receiver<ChannelEvent>) -> Self {
        Self { rx }
    }

    /// Await the next event. `None` once the broadcast channel closes.
    pub async fn recv(&mut self) -> Option<ChannelEvent> {
        loop {
            match self.rx.recv().await {
                Ok(ev) => return Some(ev),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}
