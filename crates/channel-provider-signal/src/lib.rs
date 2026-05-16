//! Signal provider for atomr-agents channels.
//!
//! This crate implements [`ChannelProvider`] by driving a local
//! [`signal-cli`](https://github.com/AsamK/signal-cli) daemon over its
//! line-delimited JSON-RPC 2.0 interface — either a Unix domain socket
//! or a TCP listener.
//!
//! ## signal-cli setup (operator-side, out of scope here)
//!
//! ```text
//! signal-cli --output=json daemon --tcp 127.0.0.1:7583
//! # or
//! signal-cli --output=json daemon --socket /tmp/signal-cli.sock
//! ```
//!
//! The account used in `SignalConfig::account` must already be
//! registered or linked through `signal-cli register` / `signal-cli
//! link`. This crate does no provisioning.
//!
//! ## Example
//!
//! ```no_run
//! use atomr_agents_channel_provider_signal::SignalProvider;
//! use serde_json::json;
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let provider = SignalProvider::from_value(json!({
//!     "endpoint": {"transport": "tcp", "address": "127.0.0.1:7583"},
//!     "account": "+15551234567",
//!     "default_channel_id": "channel-signal-demo"
//! }))?;
//! let (tx, _rx) = tokio::sync::mpsc::channel(64);
//! let handle = provider.start(tx).await?;
//! handle.signal_stop();
//! handle.join.await.ok();
//! # Ok(()) }
//! ```

#![forbid(unsafe_code)]

mod config;
mod listener;
mod rpc;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use atomr_agents_channel_core::{
    Capabilities, ChannelError, ChannelProvider, InboundMessage, OutboundMessage, ProviderAck,
    ProviderHandle, ProviderKind, Result,
};
use bytes::Bytes;
use chrono::Utc;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpStream, UnixStream};
use tokio::sync::mpsc;
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

pub use crate::config::{SignalConfig, SignalEndpoint};

/// `signal-cli` JSON-RPC provider.
///
/// Construct via [`SignalProvider::new`] or
/// [`SignalProvider::from_value`] and register the returned `Arc<dyn
/// ChannelProvider>` with the channel harness.
pub struct SignalProvider {
    config: SignalConfig,
    /// Listener handle (set once `start` has been called).
    handle: AsyncMutex<Option<crate::listener::ListenerHandle>>,
}

impl SignalProvider {
    /// Build a provider from a typed config.
    pub fn new(config: SignalConfig) -> Arc<dyn ChannelProvider> {
        Arc::new(Self {
            config,
            handle: AsyncMutex::new(None),
        })
    }

    /// Build a provider from the raw JSON `config` blob on a
    /// [`ChannelSpec`](atomr_agents_channel_core::ChannelSpec).
    pub fn from_value(value: Value) -> Result<Arc<dyn ChannelProvider>> {
        let config = SignalConfig::from_value(value)?;
        Ok(Self::new(config))
    }

    async fn connect_streams(
        &self,
    ) -> Result<(
        Box<dyn AsyncRead + Send + Unpin>,
        Box<dyn AsyncWrite + Send + Unpin>,
    )> {
        match &self.config.endpoint {
            SignalEndpoint::Tcp(addr) => {
                let stream = TcpStream::connect(addr).await.map_err(|e| {
                    ChannelError::transport(format!("signal tcp connect {addr}: {e}"))
                })?;
                let (r, w) = stream.into_split();
                Ok((Box::new(r), Box::new(w)))
            }
            SignalEndpoint::Unix(path) => {
                let stream = UnixStream::connect(path).await.map_err(|e| {
                    ChannelError::transport(format!("signal unix connect {path}: {e}"))
                })?;
                let (r, w) = stream.into_split();
                Ok((Box::new(r), Box::new(w)))
            }
        }
    }
}

#[async_trait]
impl ChannelProvider for SignalProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Signal
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            text: true,
            attachments: true,
            voice: false,
            reactions: false,
            typing: false,
            threads_native: false,
        }
    }

    async fn start(&self, inbound_tx: mpsc::Sender<InboundMessage>) -> Result<ProviderHandle> {
        let mut slot = self.handle.lock().await;
        if slot.is_some() {
            return Err(ChannelError::provider(
                "SignalProvider::start called twice without stop",
            ));
        }

        let (reader, writer) = self.connect_streams().await?;
        let stop = Arc::new(AtomicBool::new(false));
        let (handle, join) = crate::listener::spawn(
            reader,
            writer,
            self.config.default_channel_id.clone(),
            inbound_tx,
            stop.clone(),
        );
        *slot = Some(handle);

        Ok(ProviderHandle::new(stop, join))
    }

    async fn send(&self, msg: OutboundMessage) -> Result<ProviderAck> {
        let handle = {
            let slot = self.handle.lock().await;
            slot.clone()
                .ok_or_else(|| ChannelError::provider("SignalProvider not started"))?
        };

        let id = Uuid::new_v4().to_string();
        let frame = crate::rpc::build_send_request(&id, &self.config.account, &msg)?;
        let bytes = serde_json::to_vec(&frame)?;

        let rx = handle.register(id.clone());
        if let Err(e) = handle.write(bytes).await {
            handle.forget(&id);
            return Err(e);
        }

        // Wait for the matching response, with a generous default
        // timeout to avoid stalls on dead daemons.
        let value = match tokio::time::timeout(Duration::from_secs(30), rx).await {
            Ok(Ok(v)) => v,
            Ok(Err(_)) => {
                handle.forget(&id);
                return Err(ChannelError::transport(
                    "signal listener dropped before response",
                ));
            }
            Err(_) => {
                handle.forget(&id);
                return Err(ChannelError::transport("signal send: timed out"));
            }
        };

        if value
            .get("__atomr_listener_closed")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Err(ChannelError::transport(
                "signal listener closed during send",
            ));
        }

        let resp: crate::rpc::JsonRpcResponse = serde_json::from_value(value)?;
        if let Some(err) = resp.error {
            return Err(ChannelError::provider(format!(
                "signal-cli error: {}",
                err.message
            )));
        }
        let result = resp
            .result
            .ok_or_else(|| ChannelError::provider("signal-cli response missing result"))?;
        let timestamp = result
            .get("timestamp")
            .and_then(Value::as_i64)
            .ok_or_else(|| ChannelError::provider("signal-cli response missing timestamp"))?;

        Ok(ProviderAck {
            provider_msg_id: timestamp.to_string(),
            sent_at: Utc::now(),
        })
    }

    async fn fetch_media(&self, media_ref: &str) -> Result<Bytes> {
        // signal-cli stores attachments as files on the local
        // filesystem; the caller has already received the local path
        // in `media_ref` (either the raw attachment id resolved by the
        // daemon, or a path it owns). Read directly.
        let path = std::path::Path::new(media_ref);
        let data = tokio::fs::read(path).await.map_err(|e| {
            ChannelError::provider(format!("signal fetch_media {media_ref}: {e}"))
        })?;
        Ok(Bytes::from(data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_channel_core::ChannelProvider;
    use serde_json::json;

    fn make_provider() -> Arc<dyn ChannelProvider> {
        SignalProvider::from_value(json!({
            "endpoint": {"transport": "tcp", "address": "127.0.0.1:7583"},
            "account": "+15551234567",
            "default_channel_id": "signal:demo"
        }))
        .unwrap()
    }

    #[test]
    fn from_value_parses_tcp() {
        let p = make_provider();
        assert_eq!(p.kind(), ProviderKind::Signal);
    }

    #[test]
    fn from_value_parses_unix() {
        let p = SignalProvider::from_value(json!({
            "endpoint": {"transport": "unix", "address": "/tmp/signal.sock"},
            "account": "+15551234567",
            "default_channel_id": "signal:demo"
        }))
        .unwrap();
        assert_eq!(p.kind(), ProviderKind::Signal);
    }

    #[test]
    fn from_value_rejects_empty_object() {
        let res = SignalProvider::from_value(json!({}));
        match res {
            Err(ChannelError::Config(_)) => {}
            Ok(_) => panic!("expected Err, got Ok"),
            Err(e) => panic!("expected Config error, got {e:?}"),
        }
    }

    #[test]
    fn capabilities_includes_text_and_attachments() {
        let p = make_provider();
        let caps = p.capabilities();
        assert!(caps.text);
        assert!(caps.attachments);
        assert!(!caps.voice);
    }

    /// End-to-end: spin up a fake `signal-cli` over a Unix socket,
    /// have the provider connect, send one outbound message, and
    /// verify it parses the fake `timestamp` response into a
    /// `ProviderAck`. Also pushes an inbound notification and asserts
    /// it reaches the harness inbound channel.
    #[tokio::test]
    async fn integration_send_and_receive_over_unix() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::UnixListener;

        let dir = std::env::temp_dir().join(format!(
            "atomr-signal-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("signal.sock");
        // In case a previous run left the path behind.
        let _ = std::fs::remove_file(&sock);
        let listener = UnixListener::bind(&sock).unwrap();

        let sock_path = sock.to_string_lossy().to_string();
        let provider = SignalProvider::from_value(json!({
            "endpoint": {"transport": "unix", "address": sock_path},
            "account": "+15550000001",
            "default_channel_id": "signal:demo"
        }))
        .unwrap();

        // Fake signal-cli daemon: accept once, read line(s), reply with
        // a canned response, then push an inbound notification.
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (r, mut w) = stream.into_split();
            let mut lines = BufReader::new(r).lines();
            // First request: parse it and reply.
            if let Ok(Some(req)) = lines.next_line().await {
                let parsed: Value = serde_json::from_str(&req).unwrap();
                let id = parsed["id"].as_str().unwrap().to_string();
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {"timestamp": 1700000000999_i64}
                });
                let mut bytes = serde_json::to_vec(&resp).unwrap();
                bytes.push(b'\n');
                w.write_all(&bytes).await.unwrap();
                w.flush().await.unwrap();

                // Push a fake inbound `receive` notification.
                let notif = serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "receive",
                    "params": {
                        "envelope": {
                            "source": "+15559876543",
                            "sourceUuid": "peer-uuid",
                            "timestamp": 1700000000111_i64,
                            "dataMessage": {
                                "message": "ping",
                                "timestamp": 1700000000111_i64
                            }
                        }
                    }
                });
                let mut bytes = serde_json::to_vec(&notif).unwrap();
                bytes.push(b'\n');
                w.write_all(&bytes).await.unwrap();
                w.flush().await.unwrap();
            }
            // Hold the connection open until the test drops it.
            tokio::time::sleep(Duration::from_secs(5)).await;
        });

        let (in_tx, mut in_rx) = mpsc::channel(8);
        let handle = provider.start(in_tx).await.unwrap();

        let out = OutboundMessage {
            channel_id: "signal:demo".into(),
            thread_id: "thread-1".into(),
            peer: "+15559876543".into(),
            content: atomr_agents_channel_core::MessageContent::text("hello"),
            reply_to: None,
            idempotency_key: "idem-1".into(),
        };
        let ack = tokio::time::timeout(Duration::from_secs(5), provider.send(out))
            .await
            .expect("send did not complete")
            .expect("send returned error");
        assert_eq!(ack.provider_msg_id, "1700000000999");

        let inbound = tokio::time::timeout(Duration::from_secs(2), in_rx.recv())
            .await
            .expect("inbound did not arrive")
            .expect("inbound channel closed");
        assert_eq!(inbound.peer.as_str(), "peer-uuid");
        assert_eq!(inbound.provider_msg_id, "1700000000111");

        handle.signal_stop();
        let _ = tokio::time::timeout(Duration::from_secs(2), handle.join).await;
        server.abort();
        let _ = server.await;
        let _ = std::fs::remove_file(&sock);
        let _ = std::fs::remove_dir(&dir);
    }
}
