//! `TextToSpeech` impl for ElevenLabs.

use async_trait::async_trait;
use atomr_agents_stt_core::{AudioFormat, Result, SampleType, SttError, TransportKind};
use atomr_agents_stt_remote_core::{build_http_client, classify_status, retry, ws};
use atomr_agents_tts_core::{
    AudioOutput, BackendKind, Capabilities, RealtimeOptions, RealtimeSession, SynthOptions,
    SynthesisRequest, SynthesisStream, TextToSpeech, VoiceRef,
};
use reqwest::{header, Client};
use secrecy::ExposeSecret;
use serde::Serialize;

use crate::caps::CAPS;
use crate::config::ElevenLabsConfig;
use crate::stream::{ElevenLabsConvaiSession, ElevenLabsHttpStream};

pub struct ElevenLabsRunner {
    config: ElevenLabsConfig,
    client: Client,
}

impl ElevenLabsRunner {
    pub fn new(config: ElevenLabsConfig) -> Result<Self> {
        let client = build_http_client(&config.timeouts)?;
        Ok(Self { config, client })
    }

    fn auth_header(&self) -> Result<String> {
        let secret = self.config.api_key.resolve()?;
        Ok(secret.expose_secret().to_string())
    }

    fn pick_voice<'a>(&'a self, voice: &'a VoiceRef) -> Result<&'a str> {
        match voice {
            VoiceRef::Library { id } => Ok(id.as_str()),
            VoiceRef::DescribedAs { .. } => Err(SttError::UnsupportedCapability(
                "voicegen_from_text (use the /v1/voices/create endpoint instead)",
            )),
            VoiceRef::ClonedFrom(_) => Err(SttError::UnsupportedCapability(
                "voice_cloning (use the /v1/voices/add endpoint to create a clone first)",
            )),
            VoiceRef::Custom(_) => Err(SttError::UnsupportedCapability("custom_voice")),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct TtsBody<'a> {
    text: &'a str,
    model_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    voice_settings: Option<VoiceSettings>,
}

#[derive(Debug, Clone, Serialize)]
struct VoiceSettings {
    stability: f32,
    similarity_boost: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    style: Option<f32>,
}

fn voice_settings_from_opts(opts: &SynthOptions) -> Option<VoiceSettings> {
    if opts.style.is_none() && opts.pitch.is_none() && opts.rate.is_none() {
        return None;
    }
    Some(VoiceSettings {
        stability: 0.5,
        similarity_boost: 0.75,
        style: opts.style.as_ref().and_then(|_| Some(0.5)),
    })
}

fn output_format_to_audio_format(s: &str) -> AudioFormat {
    if s.starts_with("mp3") { AudioFormat::Mp3 }
    else if s.starts_with("ulaw") { AudioFormat::Mulaw { sample_rate: 8_000 } }
    else if let Some(rest) = s.strip_prefix("pcm_") {
        let sr: u32 = rest.parse().unwrap_or(24_000);
        AudioFormat::Pcm {
            sample_rate: sr,
            channels: 1,
            sample: SampleType::I16,
        }
    } else {
        AudioFormat::Mp3
    }
}

#[derive(Debug, Clone, Serialize)]
struct SfxBody<'a> {
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_seconds: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_influence: Option<f32>,
}

#[async_trait]
impl TextToSpeech for ElevenLabsRunner {
    fn capabilities(&self) -> &'static Capabilities { &CAPS }
    fn backend_kind(&self) -> BackendKind { BackendKind::ElevenLabs }
    fn transport_kind(&self) -> TransportKind { TransportKind::Hybrid }

    async fn synthesize(&self, request: SynthesisRequest) -> Result<AudioOutput> {
        match request {
            SynthesisRequest::Tts { text, voice, options } => {
                let voice_id = self.pick_voice(&voice)?.to_string();
                let model = options
                    .model
                    .clone()
                    .unwrap_or_else(|| self.config.default_model.clone());
                let chars = text.chars().count() as u32;
                let url = self
                    .config
                    .rest_endpoint
                    .join(&format!("text-to-speech/{voice_id}"))
                    .map_err(|e| SttError::internal(format!("join URL: {e}")))?;
                let auth = self.auth_header()?;
                let body = TtsBody {
                    text: &text,
                    model_id: &model,
                    voice_settings: voice_settings_from_opts(&options),
                };
                let body_bytes = serde_json::to_vec(&body)
                    .map_err(|e| SttError::internal(format!("serialize: {e}")))?;
                let policy = self.config.retry.clone();
                let client = self.client.clone();
                let output_format = self.config.default_output_format.clone();

                let bytes = retry(&policy, move || {
                    let client = client.clone();
                    let url = url.clone();
                    let auth = auth.clone();
                    let body = body_bytes.clone();
                    let output_format = output_format.clone();
                    async move {
                        let resp = client
                            .post(url)
                            .query(&[("output_format", output_format.as_str())])
                            .header("xi-api-key", &auth)
                            .header(header::CONTENT_TYPE, "application/json")
                            .body(body)
                            .send()
                            .await
                            .map_err(|e| SttError::transport(format!("11labs POST: {e}")))?;
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
                            .map_err(|e| SttError::transport(format!("11labs body: {e}")))?;
                        Ok(bytes)
                    }
                })
                .await?;

                let format = output_format_to_audio_format(&self.config.default_output_format);
                let mut out = AudioOutput::from_container(
                    bytes,
                    format,
                    0.0,
                    BackendKind::ElevenLabs,
                    chars,
                );
                out.model_id = Some(model);
                out.voice_id_used = Some(voice_id);
                Ok(out)
            }
            SynthesisRequest::SoundEffect { prompt, duration_secs, options } => {
                let chars = prompt.chars().count() as u32;
                let url = self
                    .config
                    .rest_endpoint
                    .join("sound-generation")
                    .map_err(|e| SttError::internal(format!("join URL: {e}")))?;
                let auth = self.auth_header()?;
                let prompt_influence = options
                    .extra
                    .as_ref()
                    .and_then(|v| v.get("prompt_influence"))
                    .and_then(|v| v.as_f64())
                    .map(|v| v as f32);
                let body = SfxBody {
                    text: &prompt,
                    duration_seconds: duration_secs,
                    prompt_influence,
                };
                let body_bytes = serde_json::to_vec(&body)
                    .map_err(|e| SttError::internal(format!("serialize: {e}")))?;
                let resp = self
                    .client
                    .post(url)
                    .header("xi-api-key", &auth)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(body_bytes)
                    .send()
                    .await
                    .map_err(|e| SttError::transport(format!("11labs sfx: {e}")))?;
                if !resp.status().is_success() {
                    let status = resp.status().as_u16();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(classify_status(status, None, body));
                }
                let bytes = resp
                    .bytes()
                    .await
                    .map_err(|e| SttError::transport(format!("11labs sfx body: {e}")))?;
                let format = AudioFormat::Mp3;
                let mut out = AudioOutput::from_container(
                    bytes,
                    format,
                    duration_secs.unwrap_or(0.0),
                    BackendKind::ElevenLabs,
                    chars,
                );
                out.model_id = Some("eleven_sound_effects".to_string());
                Ok(out)
            }
            SynthesisRequest::Dialogue { .. } => {
                Err(SttError::UnsupportedCapability("dialogue_multispeaker"))
            }
        }
    }

    async fn synthesize_stream(
        &self,
        request: SynthesisRequest,
    ) -> Result<Box<dyn SynthesisStream>> {
        let (text, voice_ref, options) = match request {
            SynthesisRequest::Tts { text, voice, options } => (text, voice, options),
            SynthesisRequest::SoundEffect { .. } => {
                return Err(SttError::UnsupportedCapability("sfx streaming"))
            }
            SynthesisRequest::Dialogue { .. } => {
                return Err(SttError::UnsupportedCapability("dialogue_multispeaker"))
            }
        };
        let voice_id = self.pick_voice(&voice_ref)?.to_string();
        let model = options
            .model
            .clone()
            .unwrap_or_else(|| self.config.default_model.clone());
        let url = self
            .config
            .rest_endpoint
            .join(&format!("text-to-speech/{voice_id}/stream"))
            .map_err(|e| SttError::internal(format!("join URL: {e}")))?;
        let auth = self.auth_header()?;
        let body = TtsBody {
            text: &text,
            model_id: &model,
            voice_settings: voice_settings_from_opts(&options),
        };
        let body_bytes = serde_json::to_vec(&body)
            .map_err(|e| SttError::internal(format!("serialize: {e}")))?;
        let resp = self
            .client
            .post(url)
            .query(&[("output_format", self.config.default_output_format.as_str())])
            .header("xi-api-key", &auth)
            .header(header::CONTENT_TYPE, "application/json")
            .body(body_bytes)
            .send()
            .await
            .map_err(|e| SttError::transport(format!("11labs stream POST: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(classify_status(status, None, body));
        }
        let format = output_format_to_audio_format(&self.config.default_output_format);
        Ok(Box::new(ElevenLabsHttpStream::spawn(resp.bytes_stream(), format)))
    }

    async fn open_realtime(
        &self,
        opts: RealtimeOptions,
    ) -> Result<Box<dyn RealtimeSession>> {
        let agent_id = self.config.convai_agent_id.clone().or_else(|| {
            opts.extra
                .as_ref()
                .and_then(|v| v.get("agent_id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        });
        let agent_id = agent_id.ok_or_else(|| {
            SttError::Internal(
                "Conversational AI requires an agent_id (set ELEVENLABS_AGENT_ID or pass extra={\"agent_id\": ...})".to_string(),
            )
        })?;
        let mut url = self.config.convai_endpoint.clone();
        url.query_pairs_mut().append_pair("agent_id", &agent_id);
        let auth = self.auth_header()?;
        let stream = ws::connect(url.as_str(), &[("xi-api-key", auth.as_str())]).await?;
        Ok(Box::new(ElevenLabsConvaiSession::spawn(stream)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_stt_remote_core::SecretRef;

    #[test]
    fn caps_advertise_all_five_surfaces_except_dialogue() {
        assert!(CAPS.plain_tts);
        assert!(CAPS.voicegen_from_text);
        assert!(CAPS.sound_effects);
        assert!(CAPS.realtime_bidirectional);
        assert!(CAPS.streaming_output);
        assert_eq!(CAPS.dialogue_multispeaker, None);
    }

    #[tokio::test]
    async fn dialogue_returns_unsupported() {
        let mut cfg = ElevenLabsConfig::from_env();
        cfg.api_key = SecretRef::literal("xi-test");
        let r = ElevenLabsRunner::new(cfg).unwrap();
        let req = SynthesisRequest::Dialogue {
            script: vec![],
            speakers: vec![],
            options: Default::default(),
        };
        let err = r.synthesize(req).await.unwrap_err();
        assert!(matches!(err, SttError::UnsupportedCapability("dialogue_multispeaker")));
    }
}
