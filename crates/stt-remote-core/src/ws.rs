//! `tokio-tungstenite` connection helpers.
//!
//! This module is gated on the `ws` feature. Backends that only
//! talk REST (OpenAI Whisper batch) can disable it to skip the
//! `tokio-tungstenite` dep.

use atomr_agents_stt_core::SttError;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::Request;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

pub type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Open a WebSocket connection to `url` with optional extra headers
/// (e.g. `Authorization: Token …` for Deepgram).
pub async fn connect(url: &str, headers: &[(&str, &str)]) -> Result<Ws, SttError> {
    let mut req: Request = url
        .into_client_request()
        .map_err(|e| SttError::transport(format!("ws build request: {e}")))?;
    for (k, v) in headers {
        let name = http_header_name(k)?;
        let value = v
            .parse()
            .map_err(|e| SttError::transport(format!("ws header value {v:?}: {e}")))?;
        req.headers_mut().insert(name, value);
    }
    let (stream, _resp) = connect_async(req)
        .await
        .map_err(|e| SttError::transport(format!("ws connect: {e}")))?;
    Ok(stream)
}

fn http_header_name(s: &str) -> Result<tokio_tungstenite::tungstenite::http::HeaderName, SttError> {
    s.parse()
        .map_err(|e| SttError::transport(format!("ws header name {s:?}: {e}")))
}
