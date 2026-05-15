use async_trait::async_trait;
use atomr_agents_stt_core::{Result, SttError, TransportKind};
use atomr_agents_tts_core::{
    AudioOutput, BackendKind, Capabilities, RealtimeOptions, RealtimeSession, SynthesisRequest,
    SynthesisStream, TextToSpeech,
};

use crate::caps::CAPS;
use crate::config::XttsConfig;

pub struct XttsRunner {
    config: XttsConfig,
}

impl XttsRunner {
    pub fn new(config: XttsConfig) -> Result<Self> {
        #[cfg(not(feature = "xtts-http"))]
        {
            tracing::warn!(
                "atomr-agents-tts-runtime-xtts: built without `xtts-http` feature; \
                 synthesize() will return ModelLoad errors. Rebuild with --features xtts-http \
                 once a colocated Coqui XTTS Python server is running.",
            );
        }
        Ok(Self { config })
    }
}

#[async_trait]
impl TextToSpeech for XttsRunner {
    fn capabilities(&self) -> &'static Capabilities {
        &CAPS
    }
    fn backend_kind(&self) -> BackendKind {
        BackendKind::XttsV2
    }
    fn transport_kind(&self) -> TransportKind {
        TransportKind::Hybrid
    }

    async fn synthesize(&self, _request: SynthesisRequest) -> Result<AudioOutput> {
        let _ = &self.config;
        Err(SttError::model_load(
            "atomr-agents-tts-runtime-xtts: HTTP client to colocated Coqui XTTS server not \
             yet wired in this revision. Rebuild with --features xtts-http once the binding \
             lands.",
        ))
    }

    async fn synthesize_stream(&self, _request: SynthesisRequest) -> Result<Box<dyn SynthesisStream>> {
        Err(SttError::model_load(
            "atomr-agents-tts-runtime-xtts: streaming pipeline pending; rebuild with \
             --features xtts-http.",
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
    async fn caps_advertise_zero_shot_cloning() {
        assert!(CAPS.plain_tts);
        assert!(matches!(
            CAPS.voice_cloning,
            atomr_agents_tts_core::VoiceCloningSupport::ZeroShot { .. }
        ));
        assert!(!CAPS.realtime_bidirectional);
        assert!(!CAPS.requires_network);
    }
}
