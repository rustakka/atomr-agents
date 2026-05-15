//! Server-Sent Events stream of normalized `CodingCliEvent`.

use std::convert::Infallible;
use std::time::Duration;

use atomr_agents_coding_cli_core::CodingCliEvent;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::Stream;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::AppState;

/// `GET /api/cli/runs/events` — SSE of every normalized event the
/// harness broadcasts.
pub async fn sse_events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx: broadcast::Receiver<CodingCliEvent> = state.harness.event_sender().subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|res| match res {
        Ok(ev) => {
            let json = serde_json::to_string(&ev).unwrap_or_else(|_| "null".into());
            Some(Ok(Event::default().event("coding_cli_event").data(json)))
        }
        Err(_lag) => None,
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}
