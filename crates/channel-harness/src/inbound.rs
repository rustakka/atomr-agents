use std::sync::Arc;

use atomr_agents_channel_core::{
    ChannelEvent, ChannelError, Direction, InboundMessage, MessageContent, OutboundMessage, Thread,
    ThreadId, ThreadTarget,
};
use atomr_agents_core::{CallCtx, IterationBudget, MoneyBudget, TimeBudget, TokenBudget};
use chrono::Utc;
use parking_lot::RwLock;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::harness::{record_for, HarnessInner};
use crate::outbound::OutboundJob;

pub(crate) fn spawn_inbound_loop(
    inner: Arc<HarnessInner>,
    mut rx: mpsc::Receiver<InboundMessage>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Err(e) = handle_inbound(&inner, msg).await {
                tracing::warn!(error = %e, "inbound handling failed");
                let _ = inner.event_tx.send(ChannelEvent::Error {
                    thread_id: None,
                    message_id: None,
                    reason: e.to_string(),
                });
            }
        }
    })
}

async fn handle_inbound(
    inner: &Arc<HarnessInner>,
    msg: InboundMessage,
) -> Result<(), ChannelError> {
    // Dedup by (channel, provider_msg_id).
    if inner
        .store
        .has_inbound(&msg.channel_id, &msg.provider_msg_id)
        .await?
    {
        let _ = inner.event_tx.send(ChannelEvent::MessageDuplicate {
            thread_id: msg.thread_id.clone(),
            provider_msg_id: msg.provider_msg_id.clone(),
        });
        return Ok(());
    }

    // Look up (or auto-open) the thread.
    let thread_id = ThreadId::for_peer(&msg.channel_id, &msg.peer);
    let thread_arc = match inner.threads.get(&thread_id) {
        Some(r) => r.value().clone(),
        None => {
            let Some(target) = inner.auto_open_target.clone() else {
                return Err(ChannelError::UnknownThread(thread_id.as_str().to_string()));
            };
            let mut t = Thread::new(msg.channel_id.clone(), msg.peer.clone(), target);
            t.policy = inner.default_policy;
            let arc = Arc::new(RwLock::new(t.clone()));
            inner.threads.insert(thread_id.clone(), arc.clone());
            inner.store.upsert_thread(&t).await?;
            let _ = inner.event_tx.send(ChannelEvent::ThreadOpened {
                thread_id: thread_id.clone(),
                channel_id: msg.channel_id.clone(),
                peer: msg.peer.clone(),
            });
            arc
        }
    };

    // Append inbound to thread history + store.
    {
        let mut g = thread_arc.write();
        g.push_user(&msg.content);
    }
    let inbound_record = record_for(
        &thread_id,
        Direction::Inbound,
        msg.content.clone(),
        Some(msg.provider_msg_id.clone()),
        None,
    );
    inner.store.append_message(&inbound_record).await?;

    let _ = inner.event_tx.send(ChannelEvent::MessageReceived {
        thread_id: thread_id.clone(),
        message_id: inbound_record.id.clone(),
        peer: msg.peer.clone(),
        summary: msg.content.summary(),
    });

    // Dispatch.
    let target = thread_arc.read().target.clone();
    let _ = inner.event_tx.send(ChannelEvent::TurnStarted {
        thread_id: thread_id.clone(),
        message_id: inbound_record.id.clone(),
    });

    let reply_content = match &target {
        ThreadTarget::Callable(handle) => {
            let envelope = build_envelope(&msg, &thread_arc);
            let ctx = default_ctx();
            match handle.call(envelope, ctx).await {
                Ok(value) => extract_reply(&value),
                Err(e) => {
                    let _ = inner.event_tx.send(ChannelEvent::Error {
                        thread_id: Some(thread_id.clone()),
                        message_id: Some(inbound_record.id.clone()),
                        reason: e.to_string(),
                    });
                    None
                }
            }
        }
        ThreadTarget::Harness { callable, adapter } => {
            if let Err(e) = adapter.apply(&msg).await {
                let _ = inner.event_tx.send(ChannelEvent::Error {
                    thread_id: Some(thread_id.clone()),
                    message_id: Some(inbound_record.id.clone()),
                    reason: format!("adapter.apply: {e}"),
                });
                None
            } else if adapter.one_shot() {
                match callable.call(serde_json::Value::Null, default_ctx()).await {
                    Ok(value) => adapter.reply_from_result(&value),
                    Err(e) => {
                        let _ = inner.event_tx.send(ChannelEvent::Error {
                            thread_id: Some(thread_id.clone()),
                            message_id: Some(inbound_record.id.clone()),
                            reason: e.to_string(),
                        });
                        None
                    }
                }
            } else {
                None
            }
        }
    };

    let _ = inner.event_tx.send(ChannelEvent::TurnCompleted {
        thread_id: thread_id.clone(),
        message_id: inbound_record.id.clone(),
        output_summary: reply_content
            .as_ref()
            .map(|c| c.summary())
            .unwrap_or_default(),
    });

    if let Some(content) = reply_content {
        // Append assistant message to thread history.
        thread_arc.write().push_assistant(&content);

        // Enqueue outbound.
        let attached = inner
            .providers
            .get(&msg.channel_id)
            .ok_or_else(|| ChannelError::UnknownChannel(msg.channel_id.as_str().to_string()))?;
        let outbound = OutboundMessage {
            channel_id: msg.channel_id.clone(),
            thread_id: thread_id.clone(),
            peer: msg.peer.clone(),
            content,
            reply_to: Some(msg.provider_msg_id.clone()),
            idempotency_key: format!("turn-{}-{}", inbound_record.id, Uuid::new_v4()),
        };
        let job = OutboundJob {
            outbound,
            ack: None,
        };
        if attached.outbound_tx.send(job).await.is_err() {
            let _ = inner.event_tx.send(ChannelEvent::Error {
                thread_id: Some(thread_id.clone()),
                message_id: Some(inbound_record.id),
                reason: "outbound queue closed".into(),
            });
        }
    }

    let _ = Utc::now(); // placeholder to keep the import alive
    Ok(())
}

fn build_envelope(msg: &InboundMessage, thread_arc: &Arc<RwLock<Thread>>) -> serde_json::Value {
    let (history_len, channel_id, thread_id) = {
        let g = thread_arc.read();
        (g.history.len(), g.channel.clone(), g.id.clone())
    };
    serde_json::json!({
        "user": msg.content.as_text(),
        "content": msg.content,
        "thread": { "id": thread_id.as_str(), "history_len": history_len },
        "channel": { "id": channel_id.as_str(), "peer": msg.peer.as_str() },
        "provider_msg_id": msg.provider_msg_id,
    })
}

fn extract_reply(value: &serde_json::Value) -> Option<MessageContent> {
    // Prefer { "text": "..." } envelope; fall back to top-level string;
    // also accept full MessageContent shape under "content".
    if let Some(content) = value.get("content") {
        if let Ok(c) = serde_json::from_value::<MessageContent>(content.clone()) {
            return non_empty(c);
        }
    }
    if let Some(s) = value.get("text").and_then(|v| v.as_str()) {
        return non_empty(MessageContent::text(s));
    }
    if let Some(s) = value.get("output").and_then(|v| v.as_str()) {
        return non_empty(MessageContent::text(s));
    }
    if let Some(s) = value.as_str() {
        return non_empty(MessageContent::text(s));
    }
    None
}

fn non_empty(c: MessageContent) -> Option<MessageContent> {
    if c.as_text().is_empty() {
        match &c {
            MessageContent::Attachment { .. } => Some(c),
            _ => None,
        }
    } else {
        Some(c)
    }
}

fn default_ctx() -> CallCtx {
    CallCtx {
        agent_id: None,
        tokens: TokenBudget::new(64_000),
        time: TimeBudget::new(std::time::Duration::from_secs(120)),
        money: MoneyBudget::from_usd(1.0),
        iterations: IterationBudget::new(16),
        trace: vec![],
    }
}
