//! End-to-end orchestrator smoke tests.
//!
//! Build an in-memory provider, attach it, open a thread bound to a
//! `FnCallable` that echoes inbound, push an inbound, assert outbound
//! lands on the provider's sent log.

use std::sync::Arc;

use atomr_agents_callable::FnCallable;
use atomr_agents_channel_core::memory::InMemoryProvider;
use atomr_agents_channel_core::{
    ChannelEvent, ChannelId, ChannelSpec, InboundMessage, MessageContent, PeerId, ProviderKind,
    ThreadId, ThreadTarget,
};
use atomr_agents_channel_harness::ChannelHarness;

fn echo_callable() -> Arc<dyn atomr_agents_callable::Callable> {
    Arc::new(FnCallable::labeled("echo", |input: atomr_agents_core::Value, _ctx| async move {
        let text = input
            .get("user")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        Ok(serde_json::json!({ "text": format!("echo: {text}") }))
    }))
}

#[tokio::test]
async fn inbound_routes_to_callable_and_outbound_is_sent() {
    let harness = ChannelHarness::in_memory();
    let mut events = harness.events();

    let channel_id = ChannelId::from("memory:test");
    let provider = Arc::new(InMemoryProvider::new(channel_id.clone()));
    let inbox = provider.inbox();
    let mut sent_log = provider.sent_log();

    let spec = ChannelSpec::new(channel_id.clone(), ProviderKind::Memory);
    harness
        .attach_provider(spec, provider.clone())
        .await
        .expect("attach");

    let peer = PeerId::from("alice");
    let _thread = harness
        .open_thread(
            &channel_id,
            peer.clone(),
            ThreadTarget::callable(echo_callable()),
        )
        .await
        .expect("open thread");

    let thread_id = ThreadId::for_peer(&channel_id, &peer);
    let msg = InboundMessage {
        channel_id: channel_id.clone(),
        thread_id: thread_id.clone(),
        peer: peer.clone(),
        provider_msg_id: "pmid-1".into(),
        content: MessageContent::text("hello"),
        received_at: chrono::Utc::now(),
        raw: serde_json::Value::Null,
    };
    inbox.push(msg).expect("inbox push");

    // Wait for the outbound to land.
    let outbound = tokio::time::timeout(std::time::Duration::from_secs(2), sent_log.recv())
        .await
        .expect("send timed out")
        .expect("send dropped");
    assert_eq!(outbound.content.as_text(), "echo: hello");

    // Verify event ordering at least loosely: should see thread_opened then message_received.
    let mut saw_received = false;
    let mut saw_sent = false;
    while let Ok(Some(ev)) =
        tokio::time::timeout(std::time::Duration::from_millis(200), events.recv()).await
    {
        match ev {
            ChannelEvent::MessageReceived { .. } => saw_received = true,
            ChannelEvent::MessageSent { .. } => {
                saw_sent = true;
                break;
            }
            _ => {}
        }
    }
    assert!(saw_received, "MessageReceived not observed");
    assert!(saw_sent, "MessageSent not observed");

    // Persisted records: one inbound + one outbound on the thread.
    let records = harness
        .list_messages(&thread_id, 0)
        .await
        .expect("list_messages");
    assert_eq!(records.len(), 2, "expected 2 records, got {records:?}");

    harness.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn duplicate_inbound_is_deduped() {
    let harness = ChannelHarness::in_memory();
    let channel_id = ChannelId::from("memory:dedup");
    let provider = Arc::new(InMemoryProvider::new(channel_id.clone()));
    let inbox = provider.inbox();
    harness
        .attach_provider(
            ChannelSpec::new(channel_id.clone(), ProviderKind::Memory),
            provider,
        )
        .await
        .unwrap();
    let peer = PeerId::from("bob");
    harness
        .open_thread(
            &channel_id,
            peer.clone(),
            ThreadTarget::callable(echo_callable()),
        )
        .await
        .unwrap();

    let thread_id = ThreadId::for_peer(&channel_id, &peer);
    let mk = || InboundMessage {
        channel_id: channel_id.clone(),
        thread_id: thread_id.clone(),
        peer: peer.clone(),
        provider_msg_id: "pmid-same".into(),
        content: MessageContent::text("dup"),
        received_at: chrono::Utc::now(),
        raw: serde_json::Value::Null,
    };
    inbox.push(mk()).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    inbox.push(mk()).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let recs = harness.list_messages(&thread_id, 0).await.unwrap();
    // exactly one inbound + one outbound (the second inbound was deduped)
    assert_eq!(recs.len(), 2);
}

#[tokio::test]
async fn admin_send_bypasses_target() {
    let harness = ChannelHarness::in_memory();
    let channel_id = ChannelId::from("memory:admin");
    let provider = Arc::new(InMemoryProvider::new(channel_id.clone()));
    let mut sent_log = provider.sent_log();
    harness
        .attach_provider(
            ChannelSpec::new(channel_id.clone(), ProviderKind::Memory),
            provider,
        )
        .await
        .unwrap();
    let peer = PeerId::from("carol");
    harness
        .open_thread(
            &channel_id,
            peer.clone(),
            ThreadTarget::callable(echo_callable()),
        )
        .await
        .unwrap();
    let thread_id = ThreadId::for_peer(&channel_id, &peer);

    let ack = harness
        .send(&thread_id, MessageContent::text("manual"))
        .await
        .unwrap();
    assert!(!ack.provider_msg_id.is_empty());
    let sent = tokio::time::timeout(std::time::Duration::from_millis(500), sent_log.recv())
        .await
        .expect("send")
        .expect("ack dropped");
    assert_eq!(sent.content.as_text(), "manual");
}
