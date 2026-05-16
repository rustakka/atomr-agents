//! Avatar-sink trait + lifecycle handle.
//!
//! Shape mirrors [`atomr_agents_channel_core::ProviderHandle`] so
//! supervisor patterns transfer cleanly across the framework: a
//! cooperative `Arc<AtomicBool>` stop flag and a `JoinHandle` for the
//! spawned emitter task.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::error::Result;
use crate::frame::AvatarFrame;

/// Cooperative shutdown handle for a sink's long-running emitter task.
pub struct SinkHandle {
    pub stop: Arc<AtomicBool>,
    pub join: JoinHandle<()>,
}

impl SinkHandle {
    pub fn new(stop: Arc<AtomicBool>, join: JoinHandle<()>) -> Self {
        Self { stop, join }
    }

    /// Signal stop. Callers still need to `await self.join` for the
    /// task to finish.
    pub fn signal_stop(&self) {
        self.stop
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Declarative description of which sink we're talking to. Used by
/// the harness to log + by the registry to register an avatar artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SinkKind {
    /// UDP-based custom Live Link transport (avatar-provider-livelink).
    LiveLinkUdp,
    /// NVIDIA Audio2Face-3D microservice (avatar-provider-audio2face,
    /// stub today — see FR-A2F-001).
    Audio2Face,
    /// In-process capture sink, useful for tests.
    MockCapture,
    /// Reserved for future first-party UE5 Live Link plugin transport.
    LiveLinkPlugin,
}

/// What this sink supports — used by the harness to decide what to
/// produce. Mirrors [`atomr_agents_channel_core::Capabilities`] in
/// spirit (read-only declaration, no behavior).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SinkCapabilities {
    /// Sink consumes a 52-element ARKit weight vector.
    pub emits_blendshapes: bool,
    /// Sink consumes PCM audio chunks (UE5 plays them via a
    /// `USoundWaveProcedural` on the receiver side).
    pub emits_audio: bool,
    /// Soft cap; the harness paces frames to stay at or below this.
    /// Typical: 30 (Audio2Face), 60 (Live Link).
    pub max_fps: u32,
    /// Brief description of the wire format, useful for logs.
    pub wire_format: &'static str,
}

impl Default for SinkCapabilities {
    fn default() -> Self {
        Self {
            emits_blendshapes: true,
            emits_audio: true,
            max_fps: 60,
            wire_format: "atomr-avatar-core::wire (CBOR v1)",
        }
    }
}

/// The single extension point downstream crates implement.
///
/// One sink per attached avatar. The harness owns the [`SinkHandle`]
/// it gets back from [`start`](AvatarSink::start) and signals shutdown
/// when the avatar session is torn down.
#[async_trait]
pub trait AvatarSink: Send + Sync + 'static {
    fn kind(&self) -> SinkKind;

    fn capabilities(&self) -> SinkCapabilities;

    /// Spawn the long-running emitter task. Frames flow in via
    /// `frame_rx`; the implementation drains it and emits each frame
    /// out the underlying transport. Returns a handle the harness
    /// uses to stop / await the task.
    async fn start(&self, frame_rx: mpsc::Receiver<AvatarFrame>) -> Result<SinkHandle>;
}
