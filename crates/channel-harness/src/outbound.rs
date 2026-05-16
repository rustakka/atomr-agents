use std::sync::Arc;
use std::time::Duration;

use atomr_agents_channel_core::{
    ChannelEvent, ChannelError, ChannelId, ChannelProvider, Direction, OutboundMessage,
    ProviderAck, Result,
};
use tokio::sync::{mpsc, oneshot};

use crate::harness::{record_for, HarnessInner};

pub(crate) struct OutboundJob {
    pub outbound: OutboundMessage,
    pub ack: Option<oneshot::Sender<Result<ProviderAck>>>,
}

const MAX_ATTEMPTS: u32 = 3;
const INITIAL_BACKOFF_MS: u64 = 100;
const BACKOFF_MULTIPLIER: f32 = 2.0;
const MAX_BACKOFF_MS: u64 = 4000;

pub(crate) fn spawn_outbound_worker(
    inner: Arc<HarnessInner>,
    provider: Arc<dyn ChannelProvider>,
    channel_id: ChannelId,
    mut rx: mpsc::Receiver<OutboundJob>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(job) = rx.recv().await {
            let OutboundJob { outbound, ack } = job;
            let result = process_one(&inner, &provider, &channel_id, outbound).await;
            if let Some(tx) = ack {
                let _ = tx.send(result.map(|(a, _)| a));
            } else if let Err(e) = result {
                tracing::warn!(channel = %channel_id.as_str(), error = %e, "outbound failed");
                let _ = inner.event_tx.send(ChannelEvent::Error {
                    thread_id: None,
                    message_id: None,
                    reason: e.to_string(),
                });
            }
        }
    })
}

async fn process_one(
    inner: &Arc<HarnessInner>,
    provider: &Arc<dyn ChannelProvider>,
    channel_id: &ChannelId,
    outbound: OutboundMessage,
) -> Result<(ProviderAck, String)> {
    // Idempotency: if we've already sent this `idempotency_key`, replay
    // the existing provider_msg_id without re-sending.
    if let Some(prev) = inner
        .store
        .lookup_outbound_by_key(&outbound.thread_id, &outbound.idempotency_key)
        .await?
    {
        let ack = ProviderAck {
            provider_msg_id: prev.clone(),
            sent_at: chrono::Utc::now(),
        };
        return Ok((ack, prev));
    }

    // Capability check against the channel's stored spec.
    if let Some(spec) = inner.store.get_channel(channel_id).await? {
        outbound.content.check_capabilities(&spec.capabilities)?;
    }

    // Retry loop.
    let mut delay = Duration::from_millis(INITIAL_BACKOFF_MS);
    let mut last_err: Option<ChannelError> = None;
    let ack = 'attempts: {
        for attempt in 0..MAX_ATTEMPTS {
            match provider.send(outbound.clone()).await {
                Ok(ack) => break 'attempts ack,
                Err(e) => {
                    last_err = Some(e);
                    if attempt + 1 == MAX_ATTEMPTS {
                        break;
                    }
                    tokio::time::sleep(delay).await;
                    let next_ms = (delay.as_millis() as f32 * BACKOFF_MULTIPLIER) as u64;
                    delay = Duration::from_millis(next_ms.min(MAX_BACKOFF_MS));
                }
            }
        }
        return Err(last_err
            .unwrap_or_else(|| ChannelError::transport("retry exhausted with no error")));
    };

    // Persist outbound record.
    let record = record_for(
        &outbound.thread_id,
        Direction::Outbound,
        outbound.content.clone(),
        Some(ack.provider_msg_id.clone()),
        Some(outbound.idempotency_key.clone()),
    );
    inner.store.append_message(&record).await?;

    let _ = inner.event_tx.send(ChannelEvent::MessageSent {
        thread_id: outbound.thread_id.clone(),
        message_id: record.id.clone(),
        provider_msg_id: ack.provider_msg_id.clone(),
    });

    let provider_msg_id = ack.provider_msg_id.clone();
    Ok((ack, provider_msg_id))
}
