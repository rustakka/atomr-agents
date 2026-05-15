use async_trait::async_trait;
use atomr_agents_stt_core::{Result, SttError, TransportKind};
use atomr_agents_tts_core::{
    AudioOutput, BackendKind, Capabilities, RealtimeOptions, RealtimeSession, SynthesisRequest,
    SynthesisStream, TextToSpeech, VoiceRef,
};

use crate::caps::CAPS;
use crate::config::PiperConfig;

pub struct PiperRunner {
    config: PiperConfig,
}

impl PiperRunner {
    pub fn new(config: PiperConfig) -> Result<Self> {
        #[cfg(not(feature = "piper-ort"))]
        {
            tracing::warn!(
                "atomr-agents-tts-runtime-piper: built without `piper-ort` feature; \
                 synthesize() will return ModelLoad errors. Rebuild with --features piper-ort \
                 to enable on-device inference.",
            );
        }
        Ok(Self { config })
    }

    fn pick_voice<'a>(&'a self, voice: &'a VoiceRef) -> Result<&'a str> {
        match voice {
            VoiceRef::Library { id } => Ok(id.as_str()),
            VoiceRef::DescribedAs { .. } => Err(SttError::UnsupportedCapability("voicegen_from_text")),
            VoiceRef::ClonedFrom(_) => Err(SttError::UnsupportedCapability("voice_cloning")),
            VoiceRef::Custom(_) => Err(SttError::UnsupportedCapability("custom_voice")),
        }
    }
}

#[async_trait]
impl TextToSpeech for PiperRunner {
    fn capabilities(&self) -> &'static Capabilities {
        &CAPS
    }
    fn backend_kind(&self) -> BackendKind {
        BackendKind::Piper
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
        let voice_id = self.pick_voice(&voice)?.to_string();
        let _ = (&text, &self.config);
        Err(SttError::model_load(format!(
            "atomr-agents-tts-runtime-piper: ORT pipeline not yet wired in this revision \
             (voice={voice_id}). Track upstream atomr-infer-runtime-ort completion or \
             enable --features piper-ort once the binding lands."
        )))
    }

    async fn synthesize_stream(&self, _request: SynthesisRequest) -> Result<Box<dyn SynthesisStream>> {
        Err(SttError::model_load(
            "atomr-agents-tts-runtime-piper: streaming pipeline pending; rebuild with \
             --features piper-ort once the ORT binding lands.",
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
    async fn caps_advertise_plain_tts_only() {
        assert!(CAPS.plain_tts);
        assert!(!CAPS.voicegen_from_text);
        assert!(!CAPS.sound_effects);
        assert!(!CAPS.realtime_bidirectional);
        assert!(CAPS.streaming_output);
        assert!(!CAPS.requires_network);
    }

    #[tokio::test]
    async fn synthesize_returns_model_load_without_feature() {
        let r = PiperRunner::new(PiperConfig::default()).unwrap();
        let req = SynthesisRequest::tts("hello", VoiceRef::Library { id: "en-us".into() });
        let err = r.synthesize(req).await.unwrap_err();
        assert!(matches!(err, SttError::ModelLoad(_)));
    }

    #[tokio::test]
    async fn realtime_returns_unsupported() {
        let r = PiperRunner::new(PiperConfig::default()).unwrap();
        let res = r.open_realtime(RealtimeOptions::default()).await;
        let err = match res {
            Ok(_) => panic!("expected UnsupportedCapability"),
            Err(e) => e,
        };
        assert!(matches!(
            err,
            SttError::UnsupportedCapability("realtime_bidirectional")
        ));
    }
}
