//! Server-Sent Events stream of host events.
//!
//! The host's `EventLog` is a file-only append-only JSONL sink (no in-process
//! broadcast channel), so we tail the file: start at the current end, then
//! every second emit any newly appended lines as `host_event` SSE messages.

use std::convert::Infallible;
use std::io::{Read, Seek, SeekFrom};
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::{Stream, StreamExt};

use crate::AppState;

/// `GET /api/events/stream`
pub async fn sse_events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let path = state.events.path().to_path_buf();
    // Only stream events appended after the client connects.
    let start = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

    let stream = futures::stream::unfold((path, start), |(path, mut offset)| async move {
        loop {
            tokio::time::sleep(Duration::from_millis(1000)).await;
            let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(offset);
            if len < offset {
                // file truncated / rotated — restart from the top.
                offset = 0;
            }
            if len <= offset {
                continue;
            }
            let Ok(mut f) = std::fs::File::open(&path) else {
                continue;
            };
            if f.seek(SeekFrom::Start(offset)).is_err() {
                continue;
            }
            let mut buf = String::new();
            if f.read_to_string(&mut buf).is_err() {
                continue;
            }
            offset = len;
            let events: Vec<Result<Event, Infallible>> = buf
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| Ok(Event::default().event("host_event").data(l)))
                .collect();
            if events.is_empty() {
                continue;
            }
            return Some((futures::stream::iter(events), (path, offset)));
        }
    })
    .flatten();

    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}
