//! Server-Sent Events stream of [`DeepResearchEvent`].

use std::convert::Infallible;
use std::time::Duration;

use atomr_agents_deep_research_harness::DeepResearchEvent;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::Stream;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::AppState;

/// `GET /api/research/events` — SSE stream of every event the harness
/// broadcasts on its in-process channel.
pub async fn sse_events(State(state): State<AppState>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx: broadcast::Receiver<DeepResearchEvent> = state.events.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|res| match res {
        Ok(ev) => {
            let json = serde_json::to_string(&ev).unwrap_or_else(|_| "null".into());
            Some(Ok(Event::default().event("deep_research_event").data(json)))
        }
        Err(_lag) => None,
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}
