//! The avatar harness — top-level orchestrator.
//!
//! Wires PerceptionActor → CognitionActor → SynthesisActor → SyncManager
//! → AvatarSink with cooperative shutdown. The harness is constructed
//! via [`crate::AvatarHarnessBuilder`].

use std::sync::Arc;

use atomr_agents_avatar_core::{AvatarError, AvatarFrame, AvatarSink, Result, SinkHandle};
use tokio::sync::{mpsc, Mutex};

use crate::cognition::{AgentIntentPacket, CognitionActor};
use crate::emotion::EmotionState;
use crate::perception::{PerceptionActor, Utterance};
use crate::sync_manager::{SyncBundle, SyncConfig, SyncManager};
use crate::synthesis::SynthesisActor;

/// Public harness configuration.
#[derive(Debug, Clone)]
pub struct AvatarHarnessConfig {
    pub sync: SyncConfig,
    /// Bound on the perception queue.
    pub perception_buffer: usize,
    /// Bound on the sink frame queue.
    pub frame_buffer: usize,
    /// Per-turn affect decay (see [`EmotionState::new`]).
    pub emotion_decay: f32,
}

impl Default for AvatarHarnessConfig {
    fn default() -> Self {
        Self {
            sync: SyncConfig::default(),
            perception_buffer: 32,
            frame_buffer: 512,
            emotion_decay: 0.5,
        }
    }
}

pub struct AvatarHarness {
    pub(crate) cfg: AvatarHarnessConfig,
    pub(crate) cognition: Arc<CognitionActor>,
    pub(crate) synthesis: Arc<SynthesisActor>,
    pub(crate) sync_manager: Arc<SyncManager>,
    pub(crate) emotion: EmotionState,
    pub(crate) perception: PerceptionActor,
    pub(crate) perception_rx: Mutex<Option<mpsc::Receiver<Utterance>>>,
    pub(crate) frame_tx: Mutex<Option<mpsc::Sender<AvatarFrame>>>,
    pub(crate) sink_handle: Mutex<Option<SinkHandle>>,
    pub(crate) perception_join: Mutex<Option<tokio::task::JoinHandle<()>>>,
    pub(crate) last_intent: Arc<Mutex<Option<AgentIntentPacket>>>,
}

impl AvatarHarness {
    pub(crate) fn from_parts(
        cfg: AvatarHarnessConfig,
        cognition: CognitionActor,
        synthesis: SynthesisActor,
    ) -> Self {
        let emotion = EmotionState::new(
            atomr_agents_avatar_core::EmotionVector::neutral(),
            cfg.emotion_decay,
        );
        let (perception_tx, perception_rx) = mpsc::channel(cfg.perception_buffer);
        let perception = PerceptionActor::new(perception_tx);
        let sync_manager = SyncManager::new(cfg.sync, emotion.clone());

        Self {
            cfg,
            cognition: Arc::new(cognition),
            synthesis: Arc::new(synthesis),
            sync_manager: Arc::new(sync_manager),
            emotion,
            perception,
            perception_rx: Mutex::new(Some(perception_rx)),
            frame_tx: Mutex::new(None),
            sink_handle: Mutex::new(None),
            perception_join: Mutex::new(None),
            last_intent: Arc::new(Mutex::new(None)),
        }
    }

    /// Attach a sink and spawn the long-running pipeline task.
    ///
    /// After this call, the harness will drain its perception queue
    /// in a background task: each utterance triggers cognition →
    /// synthesis → emotion update → sync-manager → sink emission.
    pub async fn attach_sink(&self, sink: Arc<dyn AvatarSink>) -> Result<()> {
        if self.sink_handle.lock().await.is_some() {
            return Err(AvatarError::sink("sink already attached"));
        }
        let (frame_tx, frame_rx) = mpsc::channel(self.cfg.frame_buffer);
        let handle = sink.start(frame_rx).await?;
        *self.sink_handle.lock().await = Some(handle);
        *self.frame_tx.lock().await = Some(frame_tx.clone());

        let rx = self
            .perception_rx
            .lock()
            .await
            .take()
            .ok_or_else(|| AvatarError::sink("perception receiver already consumed"))?;

        let cognition = self.cognition.clone();
        let synthesis = self.synthesis.clone();
        let sync_manager = self.sync_manager.clone();
        let emotion = self.emotion.clone();
        let frame_tx_for_task = frame_tx;
        let last_intent = self.last_intent.clone();

        let join = tokio::spawn(async move {
            let mut rx = rx;
            while let Some(utt) = rx.recv().await {
                if let Err(e) = handle_one_turn(
                    &cognition,
                    &synthesis,
                    &sync_manager,
                    &emotion,
                    &frame_tx_for_task,
                    &last_intent,
                    utt,
                )
                .await
                {
                    tracing::warn!(error = %e, "avatar turn failed");
                }
            }
            tracing::debug!("avatar harness pipeline task exiting (perception channel closed)");
        });
        *self.perception_join.lock().await = Some(join);
        Ok(())
    }

    /// Push a pre-transcribed user utterance through the pipeline.
    pub async fn user_said(&self, text: impl Into<String>) -> Result<()> {
        self.perception
            .push_text(text.into())
            .await
            .map_err(|e| AvatarError::perception(e.to_string()))
    }

    /// Speak text directly, bypassing cognition. Useful for canned
    /// announcements / TTS-only use cases.
    pub async fn speak_text(&self, text: impl Into<String>) -> Result<()> {
        let frame_tx = self
            .frame_tx
            .lock()
            .await
            .clone()
            .ok_or_else(|| AvatarError::sink("no sink attached"))?;
        let text = text.into();
        let synth = self.synthesis.speak(&text).await?;
        let frames = self.sync_manager.build_frames(SyncBundle {
            audio: synth.audio,
            visemes: synth.visemes,
        });
        for f in frames {
            frame_tx
                .send(f)
                .await
                .map_err(|e| AvatarError::sink(format!("frame queue closed: {e}")))?;
        }
        Ok(())
    }

    /// Snapshot the most-recent agent intent (if any turn has run).
    pub async fn last_intent(&self) -> Option<AgentIntentPacket> {
        self.last_intent.lock().await.clone()
    }

    /// Read the current running emotion state.
    pub fn emotion(&self) -> atomr_agents_avatar_core::EmotionVector {
        self.emotion.snapshot()
    }

    /// Reset the running emotion to neutral.
    pub fn reset_emotion(&self) {
        self.emotion.reset();
    }

    /// Effective per-utterance buffer config (mostly for tests).
    pub fn config(&self) -> &AvatarHarnessConfig {
        &self.cfg
    }

    /// Shut everything down. Closes the perception channel, awaits
    /// the pipeline task, then drops the frame sender so the sink
    /// task drains and exits.
    pub async fn shutdown(&self) -> Result<()> {
        // Drop perception sender by closing its receiver first.
        // We do this by dropping the (cloned) PerceptionActor: but
        // the *owned* tx is held internally — simplest: clear frame_tx
        // and signal sink stop.
        if let Some(handle) = self.sink_handle.lock().await.take() {
            handle.signal_stop();
            let _ = handle.join.await;
        }
        // Drop the cloned frame sender so the channel closes.
        *self.frame_tx.lock().await = None;
        // Abort the perception pipeline task so it doesn't hang on
        // the (still-open) perception channel.
        if let Some(j) = self.perception_join.lock().await.take() {
            j.abort();
            let _ = j.await;
        }
        Ok(())
    }
}

/// One full pipeline turn — used by the spawned task.
async fn handle_one_turn(
    cognition: &CognitionActor,
    synthesis: &SynthesisActor,
    sync_manager: &SyncManager,
    emotion: &EmotionState,
    frame_tx: &mpsc::Sender<AvatarFrame>,
    last_intent: &Arc<Mutex<Option<AgentIntentPacket>>>,
    utterance: Utterance,
) -> Result<()> {
    let intent = cognition.handle_utterance(&utterance.text).await?;
    emotion.apply(intent.emotion_delta);
    let synth = synthesis.speak(&intent.response_text).await?;
    *last_intent.lock().await = Some(intent.clone());

    let frames = sync_manager.build_frames(SyncBundle {
        audio: synth.audio,
        visemes: synth.visemes,
    });
    for f in frames {
        frame_tx
            .send(f)
            .await
            .map_err(|e| AvatarError::sink(format!("frame queue closed: {e}")))?;
    }
    let _ = utterance.speaker;
    Ok(())
}

/// Used by tests to drive the harness with a mock sink that captures
/// every emitted frame.
#[doc(hidden)]
pub mod test_support {
    use super::*;
    use atomr_agents_avatar_core::{AvatarSink, SinkCapabilities, SinkKind};

    /// In-memory sink that captures frames into a `Vec<AvatarFrame>`.
    pub struct CapturingSink {
        pub frames: Arc<Mutex<Vec<AvatarFrame>>>,
    }

    impl CapturingSink {
        pub fn new() -> Self {
            Self {
                frames: Arc::new(Mutex::new(Vec::new())),
            }
        }

        pub fn handle(&self) -> Arc<Mutex<Vec<AvatarFrame>>> {
            self.frames.clone()
        }
    }

    #[async_trait::async_trait]
    impl AvatarSink for CapturingSink {
        fn kind(&self) -> SinkKind {
            SinkKind::MockCapture
        }
        fn capabilities(&self) -> SinkCapabilities {
            SinkCapabilities::default()
        }
        async fn start(
            &self,
            mut frame_rx: mpsc::Receiver<AvatarFrame>,
        ) -> Result<SinkHandle> {
            let frames = self.frames.clone();
            let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let stop_for_task = stop.clone();
            let join = tokio::spawn(async move {
                loop {
                    if stop_for_task.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                    match tokio::time::timeout(
                        std::time::Duration::from_millis(50),
                        frame_rx.recv(),
                    )
                    .await
                    {
                        Ok(Some(f)) => frames.lock().await.push(f),
                        Ok(None) => break,
                        Err(_) => continue,
                    }
                }
            });
            Ok(SinkHandle::new(stop, join))
        }
    }
}
