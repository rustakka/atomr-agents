use async_trait::async_trait;
use atomr_agents_stt_core::{Result, SttError, TransportKind};
use atomr_agents_tts_core::{
    AudioOutput, BackendKind, Capabilities, RealtimeOptions, RealtimeSession, SynthesisRequest,
    SynthesisStream, TextToSpeech,
};

use crate::caps::CAPS;
use crate::config::MossTtsConfig;

pub struct MossTtsRunner {
    config: MossTtsConfig,
}

impl MossTtsRunner {
    pub fn new(config: MossTtsConfig) -> Result<Self> {
        #[cfg(not(feature = "moss-http"))]
        {
            tracing::warn!(
                "atomr-agents-tts-runtime-moss: built without `moss-http` feature; \
                 synthesize() will return ModelLoad errors. Rebuild with --features moss-http \
                 once a colocated MOSS-TTS server is running.",
            );
        }
        Ok(Self { config })
    }
}

#[async_trait]
impl TextToSpeech for MossTtsRunner {
    fn capabilities(&self) -> &'static Capabilities { &CAPS }
    fn backend_kind(&self) -> BackendKind { BackendKind::MossTts }
    fn transport_kind(&self) -> TransportKind { TransportKind::Hybrid }

    async fn synthesize(&self, _request: SynthesisRequest) -> Result<AudioOutput> {
        let _ = &self.config;
        Err(SttError::model_load(
            "atomr-agents-tts-runtime-moss: HTTP client to colocated MOSS-TTS server not \
             yet wired in this revision. Rebuild with --features moss-http once the binding \
             lands.",
        ))
    }

    async fn synthesize_stream(
        &self,
        _request: SynthesisRequest,
    ) -> Result<Box<dyn SynthesisStream>> {
        Err(SttError::model_load(
            "atomr-agents-tts-runtime-moss: streaming HTTP client pending; rebuild with \
             --features moss-http once the binding lands.",
        ))
    }

    async fn open_realtime(
        &self,
        _opts: RealtimeOptions,
    ) -> Result<Box<dyn RealtimeSession>> {
        Err(SttError::model_load(
            "atomr-agents-tts-runtime-moss: realtime WS client pending; rebuild with \
             --features moss-http once the binding lands.",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_tts_core::VoiceRef;

    #[tokio::test]
    async fn caps_advertise_all_five_surfaces() {
        assert!(CAPS.plain_tts);
        assert!(CAPS.voicegen_from_text);
        assert!(matches!(CAPS.voice_cloning, atomr_agents_tts_core::VoiceCloningSupport::ZeroShot { .. }));
        assert_eq!(CAPS.dialogue_multispeaker, Some(5));
        assert!(CAPS.sound_effects);
        assert!(CAPS.realtime_bidirectional);
        assert!(!CAPS.requires_network);
    }

    #[tokio::test]
    async fn synthesize_returns_model_load_without_feature() {
        let r = MossTtsRunner::new(MossTtsConfig::default()).unwrap();
        let req = SynthesisRequest::tts("hello", VoiceRef::Library { id: "default".into() });
        let err = r.synthesize(req).await.unwrap_err();
        assert!(matches!(err, SttError::ModelLoad(_)));
    }
}
