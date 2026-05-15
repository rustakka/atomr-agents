//! WebSocket relay test. Binds a real socket, pushes an event through
//! the server's broadcast channel, and asserts it arrives as JSON.

use std::sync::Arc;
use std::time::Duration;

use atomr_agents_meetings_harness::{InMemoryMeetingsStore, MeetingsHarnessEvent, MeetingsStore};
use atomr_agents_meetings_harness_web::{WebConfig, WebServer};
use atomr_agents_stt_harness::{ConversationStore, InMemoryConversationStore};
use futures_util::StreamExt;
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn ws_relays_meetings_events_as_json() {
    let analyses: Arc<dyn MeetingsStore> = Arc::new(InMemoryMeetingsStore::new());
    let transcripts: Arc<dyn ConversationStore> = Arc::new(InMemoryConversationStore::new());
    let mut config = WebConfig::default();
    // Use port 0 to let the OS pick a free port for the test.
    config.bind = "127.0.0.1:0".parse().unwrap();
    let server = WebServer::new(config, analyses, transcripts);
    let events = server.event_sender();

    let handle = server.start().await.expect("bind");
    let url = format!("ws://{}/ws", handle.bound_addr);

    let (mut socket, _resp) = tokio_tungstenite::connect_async(&url).await.expect("ws connect");

    tokio::time::sleep(Duration::from_millis(50)).await;
    events
        .send(MeetingsHarnessEvent::NoteAppended {
            note: atomr_agents_meetings_harness::Note {
                id: "n1".into(),
                text: "hello".into(),
                source_turn_indices: vec![0],
                start_ms: Some(0),
                end_ms: Some(1_000),
            },
        })
        .expect("at least one subscriber");

    let received = loop {
        let frame = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .expect("frame within timeout")
            .expect("stream open")
            .expect("frame ok");
        match frame {
            Message::Text(t) => break t,
            Message::Ping(_) | Message::Pong(_) => continue,
            other => panic!("unexpected frame: {other:?}"),
        }
    };

    let v: serde_json::Value = serde_json::from_str(&received).unwrap();
    assert_eq!(v["kind"], "note_appended");
    assert_eq!(v["note"]["text"], "hello");

    handle.shutdown().await;
}
