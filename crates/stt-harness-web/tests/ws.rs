//! WebSocket relay test. Spec row: "web — `/ws` delivers
//! `SttHarnessEvent`s". Binds a real socket, connects a client, pushes
//! an event through the server's broadcast channel, and asserts it
//! arrives serialized as JSON.

use std::sync::Arc;
use std::time::Duration;

use atomr_agents_stt_harness::{InMemoryConversationStore, SttHarnessEvent};
use atomr_agents_stt_harness_web::{WebConfig, WebServer};
use futures_util::StreamExt;
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn ws_relays_harness_events_as_json() {
    let store = Arc::new(InMemoryConversationStore::new());
    let server = WebServer::new(WebConfig::default(), store);
    let events = server.event_sender();

    let handle = server.start().await.expect("bind");
    let url = format!("ws://{}/ws", handle.bound_addr);

    let (mut socket, _resp) = tokio_tungstenite::connect_async(&url).await.expect("ws connect");

    // Give the server a moment to register the subscriber, then emit.
    tokio::time::sleep(Duration::from_millis(50)).await;
    events
        .send(SttHarnessEvent::Finished {
            reason: "stream_end".into(),
            turn_count: 2,
            total_audio_secs: 3.5,
        })
        .expect("at least one subscriber");

    // The first non-ping frame should be our event, JSON-encoded.
    let received = loop {
        let frame = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .expect("frame within timeout")
            .expect("stream open")
            .expect("frame ok");
        match frame {
            Message::Text(text) => break text,
            Message::Ping(_) | Message::Pong(_) => continue,
            other => panic!("unexpected frame: {other:?}"),
        }
    };

    let value: serde_json::Value = serde_json::from_str(&received).unwrap();
    assert_eq!(value["kind"], "finished");
    assert_eq!(value["turn_count"], 2);

    handle.shutdown().await;
}
