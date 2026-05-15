use async_trait::async_trait;
use atomr_agents_stt_core::{Result, SttError, TransportKind};
use atomr_agents_tts_core::{
    AudioOutput, BackendKind, Capabilities, RealtimeOptions, RealtimeSession, SynthesisRequest,
    SynthesisStream, TextToSpeech, VoiceRef,
};

use crate::caps::CAPS;
use crate::config::KokoroConfig;

pub struct KokoroRunner {
    config: KokoroConfig,
}

impl KokoroRunner {
    pub fn new(config: KokoroConfig) -> Result<Self> {
        #[cfg(not(feature = "kokoro-ort"))]
        {
            tracing::warn!(
                "atomr-agents-tts-runtime-kokoro: built without `kokoro-ort` feature; \
                 synthesize() will return ModelLoad errors. Rebuild with --features kokoro-ort \
                 to enable on-device inference.",
            );
        }
        Ok(Self { config })
    }

    fn pick_voice<'a>(&'a self, voice: &'a VoiceRef) -> Result<String> {
        match voice {
            VoiceRef::Library { id } => Ok(id.clone()),
            VoiceRef::DescribedAs { .. } => Err(SttError::UnsupportedCapability("voicegen_from_text")),
            VoiceRef::ClonedFrom(_) => Err(SttError::UnsupportedCapability("voice_cloning")),
            VoiceRef::Custom(_) => Err(SttError::UnsupportedCapability("custom_voice")),
        }
    }
}

#[async_trait]
impl TextToSpeech for KokoroRunner {
    fn capabilities(&self) -> &'static Capabilities {
        &CAPS
    }
    fn backend_kind(&self) -> BackendKind {
        BackendKind::Kokoro
    }
    fn transport_kind(&self) -> TransportKind {
        TransportKind::LocalModel
    }

    async fn synthesize(&self, request: SynthesisRequest) -> Result<AudioOutput> {
        let (text, voice) = match request {
            SynthesisRequest::Tts { text, voice, .. } => (text, voice),
            SynthesisRequest::SoundEffect { .. } => {
                return Err(SttError::UnsupportedCapability("sound_effects"))
            }
            SynthesisRequest::Dialogue { .. } => {
                return Err(SttError::UnsupportedCapability("dialogue_multispeaker"))
            }
        };
        let voice_id = self.pick_voice(&voice)?;
        let _ = (&text, &self.config);
        Err(SttError::model_load(format!(
            "atomr-agents-tts-runtime-kokoro: ORT pipeline not yet wired in this revision \
             (voice={voice_id}). Rebuild with --features kokoro-ort once the binding lands."
        )))
    }

    async fn synthesize_stream(&self, _request: SynthesisRequest) -> Result<Box<dyn SynthesisStream>> {
        Err(SttError::model_load(
            "atomr-agents-tts-runtime-kokoro: streaming pipeline pending; rebuild with \
             --features kokoro-ort once the ORT binding lands.",
        ))
    }

    async fn open_realtime(&self, _opts: RealtimeOptions) -> Result<Box<dyn RealtimeSession>> {
        Err(SttError::UnsupportedCapability("realtime_bidirectional"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn caps_advertise_static_voice_library() {
        assert!(CAPS.plain_tts);
        assert!(matches!(
            CAPS.voice_library,
            atomr_agents_tts_core::VoiceCatalog::Static { .. }
        ));
        assert!(!CAPS.requires_network);
    }

    #[tokio::test]
    async fn synthesize_returns_model_load_without_feature() {
        let r = KokoroRunner::new(KokoroConfig::default()).unwrap();
        let req = SynthesisRequest::tts(
            "hello",
            VoiceRef::Library {
                id: "af_bella".into(),
            },
        );
        let err = r.synthesize(req).await.unwrap_err();
        assert!(matches!(err, SttError::ModelLoad(_)));
    }
}
