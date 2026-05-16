//! End-to-end smoke test for the in-memory provider + ThreadRef::call path.

use std::sync::Arc;

use atomr_agents_callable::{Callable, FnCallable};
use atomr_agents_channel_core::{
    memory::InMemoryProvider, Capabilities, ChannelEventStream, ChannelId, ChannelProvider, ChannelSpec, MessageContent,
    PeerId, ProviderKind, Thread, ThreadRef, ThreadTarget,
};
use atomr_agents_core::{
    CallCtx, IterationBudget, MoneyBudget, TimeBudget, TokenBudget,
};

fn ctx() -> CallCtx {
    CallCtx {
        agent_id: None,
        tokens: TokenBudget::new(1000),
        time: TimeBudget::new(std::time::Duration::from_secs(5)),
        money: MoneyBudget::from_usd(1.0),
        iterations: IterationBudget::new(5),
        trace: vec![],
    }
}

#[tokio::test]
async fn thread_ref_invokes_bound_callable_with_envelope() {
    // A callable that echoes the user text back wrapped in a Value.
    let echo = Arc::new(FnCallable::new(|input: serde_json::Value, _ctx| async move {
        let user = input.get("user").and_then(|v| v.as_str()).unwrap_or("").to_string();
        Ok(serde_json::json!({"reply": user}))
    }));

    let chan = ChannelId::from("memory:integration");
    let peer = PeerId::from("alice");
    let thread = Thread::new(chan.clone(), peer.clone(), ThreadTarget::callable(echo.clone()));
    let tref = ThreadRef::new(thread);

    let out = tref.call(serde_json::json!("hello"), ctx()).await.unwrap();
    assert_eq!(out["reply"], "hello");
}

#[tokio::test]
async fn in_memory_provider_round_trips_inbound_and_outbound() {
    let chan = ChannelId::from("memory:roundtrip");
    let _spec = ChannelSpec::new(chan.clone(), ProviderKind::Memory)
        .with_capabilities(Capabilities::text_only());
    let provider = InMemoryProvider::new(chan.clone());
    let mut sent = provider.sent_log();
    let inbox = provider.inbox();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let handle = provider.start(tx).await.unwrap();

    // Push one inbound, see it forward.
    let msg = atomr_agents_channel_core::InboundMessage {
        channel_id: chan.clone(),
        thread_id: atomr_agents_channel_core::ThreadId::for_peer(&chan, &PeerId::from("alice")),
        peer: PeerId::from("alice"),
        provider_msg_id: "pmid-1".into(),
        content: MessageContent::text("ping"),
        received_at: chrono::Utc::now(),
        raw: serde_json::Value::Null,
    };
    inbox.push(msg).unwrap();
    let got = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.provider_msg_id, "pmid-1");

    // Send one outbound, see it logged.
    let _ack = provider
        .send(atomr_agents_channel_core::OutboundMessage {
            channel_id: chan.clone(),
            thread_id: got.thread_id.clone(),
            peer: PeerId::from("alice"),
            content: MessageContent::text("pong"),
            reply_to: None,
            idempotency_key: "k1".into(),
        })
        .await
        .unwrap();
    let logged = tokio::time::timeout(std::time::Duration::from_millis(200), sent.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(logged.idempotency_key, "k1");

    handle.signal_stop();
    handle.join.await.unwrap();
    // Drop unused stream just to make sure it's typed.
    let _ = ChannelEventStream::new(tokio::sync::broadcast::channel(1).1);
}
