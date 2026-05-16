//! UDP-backed [`AvatarSink`] that streams CBOR-framed
//! [`AvatarFrame`]s to an Unreal Engine 5 receiver plugin.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use atomr_agents_avatar_core::{
    encode_frame, AvatarError, AvatarFrame, AvatarSink, Result, SinkCapabilities, SinkHandle,
    SinkKind,
};

use crate::config::LiveLinkConfig;

/// UDP-based Live Link sink.
///
/// `start` spawns a tokio task that:
///
/// 1. Awaits the next [`AvatarFrame`] on the supplied mpsc receiver.
/// 2. Encodes it via [`encode_frame`] (length-prefixed CBOR).
/// 3. Ships it out the UDP socket.
/// 4. Paces itself against `cfg.max_fps`.
///
/// The task exits when either the channel closes (sender dropped) or
/// the [`SinkHandle::stop`] flag is set.
pub struct LiveLinkSink {
    cfg: LiveLinkConfig,
}

impl LiveLinkSink {
    pub fn new(cfg: LiveLinkConfig) -> Self {
        Self { cfg }
    }

    pub fn from_value(value: serde_json::Value) -> std::result::Result<Self, AvatarError> {
        Ok(Self::new(LiveLinkConfig::from_value(value)?))
    }

    /// Endpoint the sink will dial.
    pub fn target(&self) -> std::net::SocketAddr {
        self.cfg.addr
    }
}

#[async_trait]
impl AvatarSink for LiveLinkSink {
    fn kind(&self) -> SinkKind {
        SinkKind::LiveLinkUdp
    }

    fn capabilities(&self) -> SinkCapabilities {
        SinkCapabilities {
            emits_blendshapes: true,
            emits_audio: true,
            max_fps: self.cfg.max_fps,
            wire_format: "atomr-avatar-core::wire (CBOR v1, UDP length-prefixed)",
        }
    }

    async fn start(
        &self,
        mut frame_rx: mpsc::Receiver<AvatarFrame>,
    ) -> Result<SinkHandle> {
        let bind = self
            .cfg
            .bind
            .unwrap_or_else(|| "0.0.0.0:0".parse().expect("static addr parses"));
        let socket = UdpSocket::bind(bind)
            .await
            .map_err(|e| AvatarError::transport(format!("bind {bind}: {e}")))?;
        socket
            .connect(self.cfg.addr)
            .await
            .map_err(|e| AvatarError::transport(format!("connect {}: {e}", self.cfg.addr)))?;

        let target = self.cfg.addr;
        let max_fps = self.cfg.max_fps;
        let label = self.cfg.label.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_task = Arc::clone(&stop);

        let frame_budget = if max_fps == 0 {
            Duration::from_secs(0)
        } else {
            Duration::from_secs_f64(1.0 / max_fps as f64)
        };

        let join = tokio::spawn(async move {
            tracing::info!(
                target = %target,
                label = %label,
                max_fps,
                "livelink udp sink started"
            );

            let mut last_send = Instant::now() - frame_budget;
            loop {
                if stop_for_task.load(Ordering::Relaxed) {
                    tracing::debug!("livelink sink stop flag set, exiting");
                    break;
                }

                let frame = match tokio::time::timeout(
                    Duration::from_millis(100),
                    frame_rx.recv(),
                )
                .await
                {
                    Ok(Some(frame)) => frame,
                    Ok(None) => {
                        tracing::debug!("livelink sink frame channel closed, exiting");
                        break;
                    }
                    Err(_) => continue, // periodic stop-flag re-check
                };

                if frame_budget > Duration::from_secs(0) {
                    let since = last_send.elapsed();
                    if since < frame_budget {
                        tokio::time::sleep(frame_budget - since).await;
                    }
                }

                let bytes = match encode_frame(&frame) {
                    Ok(b) => b,
                    Err(e) => {
                        tracing::warn!(error = %e, "livelink encode failed; dropping frame");
                        continue;
                    }
                };

                if let Err(e) = socket.send(&bytes).await {
                    tracing::warn!(error = %e, "livelink udp send failed");
                }

                last_send = Instant::now();
            }

            tracing::info!(target = %target, label = %label, "livelink udp sink stopped");
        });

        Ok(SinkHandle::new(stop, join))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_avatar_core::{
        decode_frame, ArkitBlendshape, AudioChunk, BlendshapeWeights, SmpteTimecode,
    };
    use std::net::SocketAddr;
    use tokio::net::UdpSocket as TokioUdpSocket;

    fn frame(idx: u64) -> AvatarFrame {
        let mut w = BlendshapeWeights::zero();
        w.set(ArkitBlendshape::JawOpen, 0.5);
        AvatarFrame {
            timecode: SmpteTimecode::from_frame_index(idx, 60),
            audio: AudioChunk::empty_voice(),
            weights: w,
            emotion: None,
            body: None,
        }
    }

    #[tokio::test]
    async fn sink_streams_frames_over_udp() {
        let receiver = TokioUdpSocket::bind("127.0.0.1:0").await.unwrap();
        let recv_addr: SocketAddr = receiver.local_addr().unwrap();

        let sink = LiveLinkSink::new(LiveLinkConfig {
            addr: recv_addr,
            bind: None,
            max_fps: 0, // unpaced for the test
            label: "test".to_string(),
        });

        let (tx, rx) = mpsc::channel(8);
        let handle = sink.start(rx).await.expect("start");

        tx.send(frame(0)).await.unwrap();
        tx.send(frame(1)).await.unwrap();
        tx.send(frame(2)).await.unwrap();

        // Read three datagrams.
        let mut buf = vec![0u8; 64 * 1024];
        let mut received: Vec<AvatarFrame> = Vec::new();
        for _ in 0..3 {
            let n = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                receiver.recv(&mut buf),
            )
            .await
            .expect("timeout waiting for datagram")
            .expect("recv");
            received.push(decode_frame(&buf[..n]).expect("decode"));
        }

        handle.signal_stop();
        drop(tx);
        handle.join.await.unwrap();

        assert_eq!(received.len(), 3);
        assert_eq!(received[0].timecode.frames, 0);
        assert_eq!(received[1].timecode.frames, 1);
        assert_eq!(received[2].timecode.frames, 2);
        assert!((received[0].weights.get(ArkitBlendshape::JawOpen) - 0.5).abs() < 1e-6);
    }

    #[tokio::test]
    async fn sink_stops_when_channel_closes() {
        let receiver = TokioUdpSocket::bind("127.0.0.1:0").await.unwrap();
        let recv_addr: SocketAddr = receiver.local_addr().unwrap();

        let sink = LiveLinkSink::new(LiveLinkConfig {
            addr: recv_addr,
            bind: None,
            max_fps: 0,
            label: "test".to_string(),
        });
        let (tx, rx) = mpsc::channel(1);
        let handle = sink.start(rx).await.expect("start");

        drop(tx);

        // The join should complete promptly.
        tokio::time::timeout(std::time::Duration::from_secs(2), handle.join)
            .await
            .expect("sink did not exit after channel close")
            .unwrap();
    }
}
