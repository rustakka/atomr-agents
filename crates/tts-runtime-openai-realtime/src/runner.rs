use async_trait::async_trait;
use atomr_agents_stt_core::{Result, SttError, TransportKind};
use atomr_agents_stt_remote_core::ws;
use atomr_agents_tts_core::{
    AudioOutput, BackendKind, Capabilities, RealtimeOptions, RealtimeSession, SynthesisRequest,
    SynthesisStream, TextToSpeech,
};
use secrecy::ExposeSecret;

use crate::caps::CAPS;
use crate::config::OpenAiRealtimeConfig;
use crate::session::OpenAiRealtimeSession;

pub struct OpenAiRealtimeRunner {
    config: OpenAiRealtimeConfig,
}

impl OpenAiRealtimeRunner {
    pub fn new(config: OpenAiRealtimeConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl TextToSpeech for OpenAiRealtimeRunner {
    fn capabilities(&self) -> &'static Capabilities {
        &CAPS
    }
    fn backend_kind(&self) -> BackendKind {
        BackendKind::OpenAiRealtime
    }
    fn transport_kind(&self) -> TransportKind {
        TransportKind::WebSocket
    }

    async fn synthesize(&self, _request: SynthesisRequest) -> Result<AudioOutput> {
        Err(SttError::UnsupportedCapability(
            "openai realtime: use open_realtime() — this backend has no batch surface",
        ))
    }

    async fn synthesize_stream(&self, _request: SynthesisRequest) -> Result<Box<dyn SynthesisStream>> {
        Err(SttError::UnsupportedCapability(
            "openai realtime: use open_realtime() — this backend has no streaming-batch surface",
        ))
    }

    async fn open_realtime(&self, opts: RealtimeOptions) -> Result<Box<dyn RealtimeSession>> {
        let mut url = self.config.endpoint.clone();
        url.query_pairs_mut().append_pair("model", &self.config.model);

        let secret = self.config.api_key.resolve()?;
        let auth = format!("Bearer {}", secret.expose_secret());
        let stream = ws::connect(
            url.as_str(),
            &[("Authorization", auth.as_str()), ("OpenAI-Beta", "realtime=v1")],
        )
        .await?;

        let voice = opts
            .voice_id
            .clone()
            .unwrap_or_else(|| self.config.default_voice.clone());
        let instructions = opts
            .instructions
            .clone()
            .or_else(|| self.config.instructions.clone());
        Ok(Box::new(OpenAiRealtimeSession::spawn(
            stream,
            voice,
            instructions,
            self.config.modalities.clone(),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_stt_remote_core::SecretRef;

    #[test]
    fn caps_advertise_realtime_only() {
        assert!(!CAPS.plain_tts);
        assert!(CAPS.realtime_bidirectional);
        assert!(CAPS.streaming_output);
    }

    #[tokio::test]
    async fn synthesize_returns_unsupported() {
        let mut cfg = OpenAiRealtimeConfig::from_env();
        cfg.api_key = SecretRef::literal("sk-test");
        let r = OpenAiRealtimeRunner::new(cfg);
        let req = SynthesisRequest::tts(
            "hello",
            atomr_agents_tts_core::VoiceRef::Library { id: "alloy".into() },
        );
        let err = r.synthesize(req).await.unwrap_err();
        assert!(matches!(err, SttError::UnsupportedCapability(_)));
    }
}
