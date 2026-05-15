//! `SpeechToText` impl for the OpenAI batch endpoint.

use std::path::PathBuf;

use async_trait::async_trait;
use atomr_agents_stt_core::{
    AudioFormat, AudioInput, BackendKind, Capabilities, DiarizationSupport, Languages, Result, Segment,
    SpeechToText, StreamOptions, StreamingSession, SttError, TranscribeOptions, Transcript, TransportKind,
    Word,
};
use atomr_agents_stt_remote_core::{build_http_client, classify_status, multipart_filename_for, retry};
use bytes::Bytes;
use reqwest::multipart::{Form, Part};
use reqwest::{header, Client};
use secrecy::ExposeSecret;

use crate::config::OpenAiSttConfig;
use crate::wire::{ApiError, VerboseTranscription};

pub const CAPS: Capabilities = Capabilities {
    batch: true,
    streaming_push: false,
    realtime_microphone: false,
    diarization: DiarizationSupport::None,
    word_timestamps: true,
    utterance_timestamps: true,
    language_detection: true,
    languages: Languages::All,
    punctuation: true,
    profanity_filter: false,
    // 25 MB upload cap → ~25 min of typical mp3.
    max_audio_secs: Some(60 * 25),
    max_concurrent_streams: None,
    real_time_factor: None,
    requires_network: true,
    supported_audio_formats: &[
        AudioFormat::Wav,
        AudioFormat::Mp3,
        AudioFormat::Flac,
        AudioFormat::Ogg,
        AudioFormat::Webm,
        AudioFormat::Mp4,
    ],
    min_chunk_ms: None,
    partial_results: false,
    redaction: false,
    vad_endpointing: false,
    custom_vocabulary: false,
    cost_per_audio_min_usd: Some(0.006),
};

pub struct OpenAiSttRunner {
    config: OpenAiSttConfig,
    client: Client,
}

impl OpenAiSttRunner {
    pub fn new(config: OpenAiSttConfig) -> Result<Self> {
        let client = build_http_client(&config.timeouts)?;
        Ok(Self { config, client })
    }

    fn auth_header(&self) -> Result<String> {
        let secret = self.config.api_key.resolve()?;
        Ok(format!("Bearer {}", secret.expose_secret()))
    }

    async fn build_form(&self, input: AudioInput, opts: &TranscribeOptions) -> Result<(Form, String)> {
        let model = opts
            .model
            .clone()
            .unwrap_or_else(|| self.config.default_model.clone());

        let (bytes, format, path) = match input {
            AudioInput::File(p) => {
                let data = tokio::fs::read(&p).await?;
                let fmt = format_for_path(&p);
                (Bytes::from(data), fmt, Some(p))
            }
            AudioInput::Bytes { data, format } => (data, format, None),
            AudioInput::Pcm(_) => {
                return Err(SttError::UnsupportedFormat(
                    "OpenAI batch needs an encoded container; encode PCM to WAV via stt-audio first".into(),
                ));
            }
        };

        let filename = multipart_filename_for(path.as_deref(), &format);
        let mime = format.mime();
        let part = Part::bytes(bytes.to_vec())
            .file_name(filename.clone())
            .mime_str(mime)
            .map_err(|e| SttError::transport(format!("multipart mime: {e}")))?;

        let mut form = Form::new()
            .part("file", part)
            .text("model", model.clone())
            .text("response_format", "verbose_json");

        if let Some(lang) = opts
            .language
            .clone()
            .or_else(|| self.config.default_language.clone())
        {
            form = form.text("language", lang);
        }
        if let Some(prompt) = opts.initial_prompt.clone() {
            form = form.text("prompt", prompt);
        }
        if opts.word_timestamps_requested() {
            form = form.text("timestamp_granularities[]", "word");
        }
        Ok((form, model))
    }
}

trait OptsExt {
    fn word_timestamps_requested(&self) -> bool;
}
impl OptsExt for TranscribeOptions {
    fn word_timestamps_requested(&self) -> bool {
        // OpenAI returns word timings when `timestamp_granularities[]=word`.
        // We turn this on whenever the caller hasn't explicitly opted out
        // via `extra={"word_timestamps": false}`.
        match &self.extra {
            Some(serde_json::Value::Object(m)) => {
                m.get("word_timestamps").and_then(|v| v.as_bool()).unwrap_or(true)
            }
            _ => true,
        }
    }
}

#[async_trait]
impl SpeechToText for OpenAiSttRunner {
    fn capabilities(&self) -> &'static Capabilities {
        &CAPS
    }

    fn backend_kind(&self) -> BackendKind {
        BackendKind::OpenAi
    }

    fn transport_kind(&self) -> TransportKind {
        TransportKind::Rest
    }

    async fn transcribe(&self, input: AudioInput, opts: TranscribeOptions) -> Result<Transcript> {
        if opts.diarize {
            return Err(SttError::UnsupportedCapability("diarization"));
        }
        let url = self
            .config
            .endpoint
            .join("audio/transcriptions")
            .map_err(|e| SttError::internal(format!("join URL: {e}")))?;
        let auth = self.auth_header()?;
        let org = self.config.organization.clone();

        // We can't easily clone Form, so the retry helper rebuilds
        // the form per attempt by re-running build_form (which only
        // re-allocates the byte vec, no re-read of disk for the
        // already-loaded bytes case).
        let policy = self.config.retry.clone();
        let client = self.client.clone();
        let url = url.clone();
        let runner_input = input;
        let runner_opts = opts;
        let (verbose, model_used) = retry(&policy, move || {
            let client = client.clone();
            let url = url.clone();
            let auth = auth.clone();
            let org = org.clone();
            // We need to rebuild the form per attempt: reqwest::Form
            // is not Clone. The byte payload in `runner_input` /
            // `runner_opts` IS Clone (Bytes is cheap-clone, PathBuf
            // re-reads on each attempt only for AudioInput::File).
            let input_clone = runner_input.clone();
            let opts_clone = runner_opts.clone();
            let this = self;
            async move {
                let (form, model) = this.build_form(input_clone, &opts_clone).await?;
                let mut req = client.post(url.clone()).header(header::AUTHORIZATION, &auth);
                if let Some(org) = &org {
                    req = req.header("OpenAI-Organization", org);
                }
                let resp = req
                    .multipart(form)
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
                    let msg = serde_json::from_str::<ApiError>(&body)
                        .map(|e| e.message().to_string())
                        .unwrap_or(body);
                    return Err(classify_status(status, retry_after, msg));
                }

                let parsed: VerboseTranscription = resp
                    .json()
                    .await
                    .map_err(|e| SttError::transport(format!("openai parse: {e}")))?;
                Ok((parsed, model))
            }
        })
        .await?;

        Ok(into_transcript(verbose, model_used))
    }

    async fn open_stream(&self, _opts: StreamOptions) -> Result<Box<dyn StreamingSession>> {
        Err(SttError::UnsupportedCapability("streaming_push"))
    }
}

fn format_for_path(p: &PathBuf) -> AudioFormat {
    match p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("wav") => AudioFormat::Wav,
        Some("mp3") => AudioFormat::Mp3,
        Some("flac") => AudioFormat::Flac,
        Some("ogg") | Some("oga") => AudioFormat::Ogg,
        Some("opus") => AudioFormat::Opus,
        Some("webm") => AudioFormat::Webm,
        Some("mp4") | Some("m4a") => AudioFormat::Mp4,
        Some("aac") => AudioFormat::Aac,
        _ => AudioFormat::Wav,
    }
}

fn into_transcript(v: VerboseTranscription, model: String) -> Transcript {
    let duration = v.duration.unwrap_or(0.0);
    let language = v.language;
    let mut segments: Vec<Segment> = v
        .segments
        .into_iter()
        .map(|s| Segment {
            text: s.text,
            start_ms: (s.start * 1000.0) as u32,
            end_ms: (s.end * 1000.0) as u32,
            words: Vec::new(),
            speaker: None,
            confidence: s.avg_logprob,
        })
        .collect();
    if !v.words.is_empty() && !segments.is_empty() {
        // OpenAI returns words at the top level (not per-segment).
        // Distribute words to the segment whose [start,end] window
        // contains them; fallback to the first segment.
        for w in v.words {
            let word = Word {
                text: w.word,
                start_ms: (w.start * 1000.0) as u32,
                end_ms: (w.end * 1000.0) as u32,
                confidence: None,
            };
            let idx = segments
                .iter()
                .position(|seg| word.start_ms >= seg.start_ms && word.start_ms < seg.end_ms)
                .unwrap_or(0);
            segments[idx].words.push(word);
        }
    }
    Transcript {
        text: v.text,
        language,
        segments,
        duration_secs: duration,
        backend: BackendKind::OpenAi,
        model_id: Some(model),
        cost_usd: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OPENAI_BASE_URL;
    use atomr_agents_stt_remote_core::SecretRef;

    #[test]
    fn caps_round_trip() {
        let v = serde_json::to_value(&CAPS).unwrap();
        assert_eq!(v["batch"], true);
        assert_eq!(v["streaming_push"], false);
        assert_eq!(v["diarization"], "none");
    }

    #[test]
    fn config_from_env_uses_default_endpoint() {
        let c = OpenAiSttConfig::from_env();
        assert_eq!(c.endpoint.as_str(), OPENAI_BASE_URL);
        assert_eq!(c.default_model, "whisper-1");
    }

    #[test]
    fn unsupported_diarize_surfaces_typed_error() {
        // Build a runner with a literal key so we don't require env.
        let mut cfg = OpenAiSttConfig::from_env();
        cfg.api_key = SecretRef::literal("sk-test");
        let r = OpenAiSttRunner::new(cfg).unwrap();
        let opts = TranscribeOptions {
            diarize: true,
            ..Default::default()
        };
        let err = futures::executor::block_on(r.transcribe(
            AudioInput::Bytes {
                data: Bytes::from_static(b"x"),
                format: AudioFormat::Wav,
            },
            opts,
        ))
        .unwrap_err();
        assert!(matches!(err, SttError::UnsupportedCapability("diarization")));
    }
}
