//! Builder for [`crate::AvatarHarness`].

use std::sync::Arc;

use atomr_agents_avatar_core::{AvatarError, Result};
use atomr_agents_tts_core::{DynTextToSpeech, VoiceRef};

use crate::cognition::{AvatarInferenceClient, CognitionActor, CognitionConfig};
use crate::harness::{AvatarHarness, AvatarHarnessConfig};
use crate::synthesis::SynthesisActor;

#[derive(Default)]
pub struct AvatarHarnessBuilder {
    cfg: AvatarHarnessConfig,
    inference: Option<Arc<dyn AvatarInferenceClient>>,
    cognition_cfg: Option<CognitionConfig>,
    tts: Option<DynTextToSpeech>,
    voice: Option<VoiceRef>,
}

impl AvatarHarnessBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_config(mut self, cfg: AvatarHarnessConfig) -> Self {
        self.cfg = cfg;
        self
    }

    pub fn with_inference(mut self, client: Arc<dyn AvatarInferenceClient>) -> Self {
        self.inference = Some(client);
        self
    }

    pub fn with_cognition_config(mut self, cfg: CognitionConfig) -> Self {
        self.cognition_cfg = Some(cfg);
        self
    }

    pub fn with_tts(mut self, tts: DynTextToSpeech, voice: VoiceRef) -> Self {
        self.tts = Some(tts);
        self.voice = Some(voice);
        self
    }

    /// Build the harness. Returns an error if `inference` or `tts`
    /// haven't been supplied.
    pub fn build(self) -> Result<AvatarHarness> {
        let inference = self
            .inference
            .ok_or_else(|| AvatarError::config("inference client is required"))?;
        let tts = self
            .tts
            .ok_or_else(|| AvatarError::config("tts backend is required"))?;
        let voice = self
            .voice
            .ok_or_else(|| AvatarError::config("voice reference is required"))?;

        let cognition_cfg = self.cognition_cfg.unwrap_or_default();
        let cognition = CognitionActor::new(inference, cognition_cfg);
        let synthesis = SynthesisActor::new(tts, voice);

        Ok(AvatarHarness::from_parts(self.cfg, cognition, synthesis))
    }
}
