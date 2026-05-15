//! `TextToSpeech` impl for OpenAI's `/v1/audio/speech` endpoint.

use async_trait::async_trait;
use atomr_agents_stt_core::{AudioFormat, Result, SampleType, SttError, TransportKind};
use atomr_agents_stt_remote_core::{build_http_client, classify_status, retry};
use atomr_agents_tts_core::{
    AudioOutput, BackendKind, Capabilities, RealtimeOptions, RealtimeSession, SynthOptions, SynthesisRequest,
    SynthesisStream, TextToSpeech, VoiceRef,
};
use reqwest::{header, Client};
use secrecy::ExposeSecret;
use serde::Serialize;

use crate::caps::CAPS;
use crate::config::OpenAiTtsConfig;
use crate::stream::OpenAiSynthesisStream;

pub struct OpenAiTtsRunner {
    config: OpenAiTtsConfig,
    client: Client,
}

impl OpenAiTtsRunner {
    pub fn new(config: OpenAiTtsConfig) -> Result<Self> {
        let client = build_http_client(&config.timeouts)?;
        Ok(Self { config, client })
    }

    fn auth_header(&self) -> Result<String> {
        let secret = self.config.api_key.resolve()?;
        Ok(format!("Bearer {}", secret.expose_secret()))
    }

    fn pick_voice<'a>(&'a self, voice: &'a VoiceRef) -> Result<&'a str> {
        match voice {
            VoiceRef::Library { id } => Ok(id.as_str()),
            VoiceRef::DescribedAs { .. } => Err(SttError::UnsupportedCapability("voicegen_from_text")),
            VoiceRef::ClonedFrom(_) => Err(SttError::UnsupportedCapability("voice_cloning")),
            VoiceRef::Custom(_) => Err(SttError::UnsupportedCapability("custom_voice")),
        }
    }

    fn pick_format(&self, opts: &SynthOptions) -> String {
        if let Some(fmt) = opts.format.as_ref() {
            return format_to_str(fmt).to_string();
        }
        self.config.default_format.clone()
    }

    fn pick_model<'a>(&'a self, opts: &'a SynthOptions) -> &'a str {
        opts.model
            .as_deref()
            .unwrap_or(self.config.default_model.as_str())
    }
}

#[derive(Debug, Clone, Serialize)]
struct SpeechPayload<'a> {
    model: &'a str,
    input: &'a str,
    voice: &'a str,
    response_format: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
}

fn format_to_str(f: &AudioFormat) -> &'static str {
    match f {
        AudioFormat::Mp3 => "mp3",
        AudioFormat::Opus => "opus",
        AudioFormat::Aac => "aac",
        AudioFormat::Flac => "flac",
        AudioFormat::Wav => "wav",
        AudioFormat::Pcm { .. } => "pcm",
        _ => "mp3",
    }
}

fn format_from_str(s: &str) -> AudioFormat {
    match s {
        "opus" => AudioFormat::Opus,
        "aac" => AudioFormat::Aac,
        "flac" => AudioFormat::Flac,
        "wav" => AudioFormat::Wav,
        "pcm" => AudioFormat::Pcm {
            sample_rate: 24_000,
            channels: 1,
            sample: SampleType::I16,
        },
        _ => AudioFormat::Mp3,
    }
}

#[async_trait]
impl TextToSpeech for OpenAiTtsRunner {
    fn capabilities(&self) -> &'static Capabilities {
        &CAPS
    }
    fn backend_kind(&self) -> BackendKind {
        BackendKind::OpenAi
    }
    fn transport_kind(&self) -> TransportKind {
        TransportKind::Rest
    }

    async fn synthesize(&self, request: SynthesisRequest) -> Result<AudioOutput> {
        let (text, voice_ref, opts) = match request {
            SynthesisRequest::Tts { text, voice, options } => (text, voice, options),
            SynthesisRequest::SoundEffect { .. } => {
                return Err(SttError::UnsupportedCapability("sound_effects"))
            }
            SynthesisRequest::Dialogue { .. } => {
                return Err(SttError::UnsupportedCapability("dialogue_multispeaker"))
            }
        };
        let voice = self.pick_voice(&voice_ref)?.to_string();
        let model = self.pick_model(&opts).to_string();
        let response_format = self.pick_format(&opts);
        let speed = opts.rate.or(self.config.default_speed);
        let instructions = opts.style.clone();
        let chars = text.chars().count() as u32;

        let url = self
            .config
            .endpoint
            .join("audio/speech")
            .map_err(|e| SttError::internal(format!("join URL: {e}")))?;
        let auth = self.auth_header()?;
        let org = self.config.organization.clone();
        let policy = self.config.retry.clone();
        let client = self.client.clone();

        let response_format_for_format = response_format.clone();
        let bytes = retry(&policy, move || {
            let client = client.clone();
            let url = url.clone();
            let auth = auth.clone();
            let org = org.clone();
            let model = model.clone();
            let voice = voice.clone();
            let response_format = response_format.clone();
            let text = text.clone();
            let instructions = instructions.clone();
            async move {
                let payload = SpeechPayload {
                    model: &model,
                    input: &text,
                    voice: &voice,
                    response_format: &response_format,
                    speed,
                    instructions,
                };
                let body = serde_json::to_vec(&payload)
                    .map_err(|e| SttError::internal(format!("serialize: {e}")))?;
                let mut req = client
                    .post(url)
                    .header(header::AUTHORIZATION, &auth)
                    .header(header::CONTENT_TYPE, "application/json");
                if let Some(org) = &org {
                    req = req.header("OpenAI-Organization", org);
                }
                let resp = req
                    .body(body)
                    .send()
                    .await
                    .map_err(|e| SttError::transport(format!("openai POST: {e}")))?;
                let status = resp.status().as_u16();
                if !resp.status().is_success() {
                    let retry_after = resp
                        .headers()
                        .get(header::RETRY_AFTER)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok());
                    let body = resp.text().await.unwrap_or_default();
                    return Err(classify_status(status, retry_after, body));
                }
                let bytes = resp
                    .bytes()
                    .await
                    .map_err(|e| SttError::transport(format!("openai read body: {e}")))?;
                Ok(bytes)
            }
        })
        .await?;

        // OpenAI returns the encoded container (mp3 / opus / aac /
        // flac / wav / pcm). Surface as `container_bytes` so callers
        // can decide whether to decode or store.
        let format = format_from_str(&response_format_for_format);
        let mut out = AudioOutput::from_container(bytes, format, 0.0, BackendKind::OpenAi, chars);
        out.model_id = Some(self.pick_model(&opts).to_string());
        out.voice_id_used = Some(self.pick_voice(&voice_ref)?.to_string());
        Ok(out)
    }

    async fn synthesize_stream(&self, request: SynthesisRequest) -> Result<Box<dyn SynthesisStream>> {
        let (text, voice_ref, opts) = match request {
            SynthesisRequest::Tts { text, voice, options } => (text, voice, options),
            SynthesisRequest::SoundEffect { .. } => {
                return Err(SttError::UnsupportedCapability("sound_effects"))
            }
            SynthesisRequest::Dialogue { .. } => {
                return Err(SttError::UnsupportedCapability("dialogue_multispeaker"))
            }
        };
        let voice = self.pick_voice(&voice_ref)?.to_string();
        let model = self.pick_model(&opts).to_string();
        let response_format = self.pick_format(&opts);
        let speed = opts.rate.or(self.config.default_speed);
        let instructions = opts.style.clone();

        let url = self
            .config
            .endpoint
            .join("audio/speech")
            .map_err(|e| SttError::internal(format!("join URL: {e}")))?;
        let auth = self.auth_header()?;
        let payload = SpeechPayload {
            model: &model,
            input: &text,
            voice: &voice,
            response_format: &response_format,
            speed,
            instructions,
        };
        let body = serde_json::to_vec(&payload).map_err(|e| SttError::internal(format!("serialize: {e}")))?;
        let mut req = self
            .client
            .post(url)
            .header(header::AUTHORIZATION, &auth)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(org) = &self.config.organization {
            req = req.header("OpenAI-Organization", org);
        }
        let resp = req
            .body(body)
            .send()
            .await
            .map_err(|e| SttError::transport(format!("openai POST: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(classify_status(status, None, body));
        }
        let format = format_from_str(&response_format);
        let body_stream = resp.bytes_stream();
        Ok(Box::new(OpenAiSynthesisStream::spawn(body_stream, format)))
    }

    async fn open_realtime(&self, _opts: RealtimeOptions) -> Result<Box<dyn RealtimeSession>> {
        Err(SttError::UnsupportedCapability(
            "realtime_bidirectional (use atomr-agents-tts-runtime-openai-realtime instead)",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_stt_remote_core::SecretRef;

    #[test]
    fn caps_advertise_streaming_no_realtime() {
        assert!(CAPS.plain_tts);
        assert!(CAPS.streaming_output);
        assert!(!CAPS.realtime_bidirectional);
    }

    #[tokio::test]
    async fn unsupported_dialogue_returns_typed_error() {
        let mut cfg = OpenAiTtsConfig::from_env();
        cfg.api_key = SecretRef::literal("sk-test");
        let r = OpenAiTtsRunner::new(cfg).unwrap();
        let req = SynthesisRequest::Dialogue {
            script: vec![],
            speakers: vec![],
            options: Default::default(),
        };
        let err = r.synthesize(req).await.unwrap_err();
        assert!(matches!(
            err,
            SttError::UnsupportedCapability("dialogue_multispeaker")
        ));
    }

    #[tokio::test]
    async fn unsupported_voicegen_returns_typed_error() {
        let mut cfg = OpenAiTtsConfig::from_env();
        cfg.api_key = SecretRef::literal("sk-test");
        let r = OpenAiTtsRunner::new(cfg).unwrap();
        let req = SynthesisRequest::tts("hello", VoiceRef::described("warm and slow"));
        let err = r.synthesize(req).await.unwrap_err();
        assert!(matches!(
            err,
            SttError::UnsupportedCapability("voicegen_from_text")
        ));
    }
}
