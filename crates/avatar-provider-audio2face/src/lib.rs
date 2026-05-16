//! Avatar Audio2Face provider — **stub**.
//!
//! The real runtime delegates to NVIDIA Audio2Face-3D via
//! atomr-infer's audio→ARKit-blendshape modality. That modality does
//! not yet exist upstream; see
//! `docs/upstream-feature-requests/atomr-infer-audio2face.md`
//! (FR-A2F-001) for the proposed `RuntimeKind::Audio2Face` and
//! batch shape.
//!
//! Until FR-A2F-001 lands, [`Audio2FaceSink::new`] returns
//! [`Audio2FaceError::Blocked`] — callers should fall back to
//! ElevenLabs character-alignment + the `avatar-core::viseme_to_arkit`
//! table for lipsync. The sink trait is wired now so the swap is
//! mechanical once the upstream runtime is available.
//!
//! **x86_64 + NVIDIA-GPU only** at the deployment layer. The crate
//! itself only compiles on x86_64 via the `#![cfg]` below so workspace
//! builds on aarch64 don't fail.

#![cfg(target_arch = "x86_64")]
#![forbid(unsafe_code)]

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_avatar_core::{
    AvatarError, AvatarFrame, AvatarSink, Result, SinkCapabilities, SinkHandle, SinkKind,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;

/// Configuration for the Audio2Face sink (forward-compat — fields are
/// the ones the upstream FR will need to wire through).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Audio2FaceConfig {
    /// gRPC endpoint of the Audio2Face-3D microservice. Once
    /// FR-A2F-001 lands upstream, this will be passed through to
    /// `atomr_infer_core::RuntimeConfig::Audio2Face`.
    pub grpc_endpoint: String,
    /// Optional emotion preset name the A2F service supports
    /// (`"neutral"`, `"happy"`, `"angry"`, etc.).
    #[serde(default)]
    pub emotion_preset: Option<String>,
    /// Multiplier applied to all 52 blendshape weights before
    /// emission (A2F's `AnimationHeader.multiplier`).
    #[serde(default = "default_multiplier")]
    pub blendshape_multiplier: f32,
}

fn default_multiplier() -> f32 {
    1.0
}

#[derive(Debug, Error)]
pub enum Audio2FaceError {
    /// The Audio2Face modality is not yet available in atomr-infer.
    /// Tracking: `docs/upstream-feature-requests/atomr-infer-audio2face.md`.
    #[error("audio2face modality is blocked on atomr-infer FR-A2F-001 (see docs/upstream-feature-requests/atomr-infer-audio2face.md)")]
    Blocked,
}

impl From<Audio2FaceError> for AvatarError {
    fn from(e: Audio2FaceError) -> Self {
        AvatarError::Unsupported(e.to_string())
    }
}

/// Placeholder sink for the Audio2Face → MetaHuman pipeline. Until the
/// upstream runtime exists, constructing one of these errors with
/// [`Audio2FaceError::Blocked`]. The trait is wired so swap-in is a
/// one-line change in the harness once FR-A2F-001 ships.
pub struct Audio2FaceSink {
    _cfg: Audio2FaceConfig,
}

impl Audio2FaceSink {
    pub fn new(cfg: Audio2FaceConfig) -> std::result::Result<Self, Audio2FaceError> {
        // Intentional: keep the no-op shape obvious so a future FR
        // implementer just replaces this guard with the real client.
        let _ = cfg;
        Err(Audio2FaceError::Blocked)
    }

    pub fn from_value(value: serde_json::Value) -> std::result::Result<Self, AvatarError> {
        let cfg: Audio2FaceConfig =
            serde_json::from_value(value).map_err(|e| AvatarError::Config(e.to_string()))?;
        Self::new(cfg).map_err(Into::into)
    }
}

#[async_trait]
impl AvatarSink for Audio2FaceSink {
    fn kind(&self) -> SinkKind {
        SinkKind::Audio2Face
    }

    fn capabilities(&self) -> SinkCapabilities {
        SinkCapabilities {
            emits_blendshapes: true,
            emits_audio: true,
            max_fps: 30,
            wire_format: "atomr-infer::audio2face (blocked on FR-A2F-001)",
        }
    }

    async fn start(&self, _frame_rx: mpsc::Receiver<AvatarFrame>) -> Result<SinkHandle> {
        Err(AvatarError::Unsupported(
            "audio2face sink is not yet implemented — see FR-A2F-001".into(),
        ))
    }
}

#[doc(hidden)]
pub fn _stop_flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}
