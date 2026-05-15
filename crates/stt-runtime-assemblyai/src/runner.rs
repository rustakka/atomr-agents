//! `SpeechToText` impl for AssemblyAI.

use std::time::Duration;

use async_trait::async_trait;
use atomr_agents_stt_core::{
    AudioInput, BackendKind, Capabilities, Result, Segment, SpeakerTag, SpeechToText, StreamOptions,
    StreamingSession, SttError, TranscribeOptions, Transcript, TransportKind, Word,
};
use atomr_agents_stt_remote_core::{build_http_client, classify_status, retry, ws};
use bytes::Bytes;
use reqwest::{header, Client};
use secrecy::ExposeSecret;
use tokio::time::sleep;

use crate::caps::CAPS;
use crate::config::AssemblyAiConfig;
use crate::stream::AssemblyStreamingSession;
use crate::wire::{AssemblyWord, CreateTranscriptRequest, TranscriptResult, TranscriptStub, UploadResponse};

pub struct AssemblyAiRunner {
    config: AssemblyAiConfig,
    client: Client,
}

impl AssemblyAiRunner {
    pub fn new(config: AssemblyAiConfig) -> Result<Self> {
        let client = build_http_client(&config.timeouts)?;
        Ok(Self { config, client })
    }

    fn auth_header(&self) -> Result<String> {
        let secret = self.config.api_key.resolve()?;
        Ok(secret.expose_secret().to_string())
    }
}

#[async_trait]
impl SpeechToText for AssemblyAiRunner {
    fn capabilities(&self) -> &'static Capabilities {
        &CAPS
    }

    fn backend_kind(&self) -> BackendKind {
        BackendKind::AssemblyAi
    }

    fn transport_kind(&self) -> TransportKind {
        TransportKind::Hybrid
    }

    async fn transcribe(&self, input: AudioInput, opts: TranscribeOptions) -> Result<Transcript> {
        let bytes = read_input(input).await?;
        let auth = self.auth_header()?;

        // 1) Upload bytes → upload_url.
        let upload_url = self
            .config
            .rest_endpoint
            .join("upload")
            .map_err(|e| SttError::internal(format!("join URL: {e}")))?;
        let policy = self.config.retry.clone();
        let client = self.client.clone();

        let upload: UploadResponse = retry(&policy, move || {
            let client = client.clone();
            let url = upload_url.clone();
            let auth = auth.clone();
            let body = bytes.clone();
            async move {
                let resp = client
                    .post(url)
                    .header("authorization", &auth)
                    .header(header::CONTENT_TYPE, "application/octet-stream")
                    .body(body.to_vec())
                    .send()
                    .await
                    .map_err(|e| SttError::transport(format!("assembly upload: {e}")))?;
                if !resp.status().is_success() {
                    let status = resp.status().as_u16();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(classify_status(status, None, body));
                }
                resp.json::<UploadResponse>()
                    .await
                    .map_err(|e| SttError::transport(format!("assembly upload parse: {e}")))
            }
        })
        .await?;

        // 2) Create transcript job.
        let auth = self.auth_header()?;
        let create_url = self
            .config
            .rest_endpoint
            .join("transcript")
            .map_err(|e| SttError::internal(format!("join URL: {e}")))?;
        let model = opts
            .model
            .clone()
            .unwrap_or_else(|| self.config.default_model.clone());
        let language = opts
            .language
            .clone()
            .or_else(|| self.config.default_language.clone());
        let want_speaker_labels = opts.diarize || self.config.default_speaker_labels;
        let req = CreateTranscriptRequest {
            audio_url: &upload.upload_url,
            speech_model: Some(&model),
            language_code: language.as_deref(),
            speaker_labels: want_speaker_labels,
            punctuate: opts.punctuation,
            format_text: true,
        };
        let stub = self
            .client
            .post(create_url)
            .header("authorization", &auth)
            .json(&req)
            .send()
            .await
            .map_err(|e| SttError::transport(format!("assembly create: {e}")))?;
        if !stub.status().is_success() {
            let status = stub.status().as_u16();
            let body = stub.text().await.unwrap_or_default();
            return Err(classify_status(status, None, body));
        }
        let stub: TranscriptStub = stub
            .json()
            .await
            .map_err(|e| SttError::transport(format!("assembly create parse: {e}")))?;

        // 3) Poll until completed / error. Backoff capped at 5s.
        let poll_url = self
            .config
            .rest_endpoint
            .join(&format!("transcript/{}", stub.id))
            .map_err(|e| SttError::internal(format!("join URL: {e}")))?;
        let mut delay = Duration::from_millis(250);
        loop {
            let resp = self
                .client
                .get(poll_url.clone())
                .header("authorization", &auth)
                .send()
                .await
                .map_err(|e| SttError::transport(format!("assembly poll: {e}")))?;
            if !resp.status().is_success() {
                let status = resp.status().as_u16();
                let body = resp.text().await.unwrap_or_default();
                return Err(classify_status(status, None, body));
            }
            let result: TranscriptResult = resp
                .json()
                .await
                .map_err(|e| SttError::transport(format!("assembly poll parse: {e}")))?;
            match result.status.as_str() {
                "queued" | "processing" => {
                    sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(5));
                    continue;
                }
                "completed" => return Ok(into_transcript(result)),
                "error" => {
                    return Err(SttError::Backend {
                        status: 0,
                        message: result
                            .error
                            .unwrap_or_else(|| "assembly transcript failed".into()),
                    })
                }
                other => {
                    return Err(SttError::internal(format!(
                        "assembly: unknown transcript status {other:?}"
                    )))
                }
            }
        }
    }

    async fn open_stream(&self, opts: StreamOptions) -> Result<Box<dyn StreamingSession>> {
        let mut url = self.config.ws_endpoint.clone();
        let sample_rate = match &opts.format {
            Some(atomr_agents_stt_core::AudioFormat::Pcm { sample_rate, .. }) => *sample_rate,
            _ => 16_000,
        };
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("sample_rate", &sample_rate.to_string());
            q.append_pair("encoding", "pcm_s16le");
            q.append_pair("format_turns", "true");
            if let Some(lang) = opts
                .language
                .clone()
                .or_else(|| self.config.default_language.clone())
            {
                q.append_pair("language_code", &lang);
            }
        }
        let auth = self.auth_header()?;
        let stream = ws::connect(url.as_str(), &[("Authorization", auth.as_str())]).await?;
        Ok(Box::new(AssemblyStreamingSession::spawn(stream)))
    }
}

async fn read_input(input: AudioInput) -> Result<Bytes> {
    match input {
        AudioInput::File(p) => Ok(Bytes::from(tokio::fs::read(&p).await?)),
        AudioInput::Bytes { data, .. } => Ok(data),
        AudioInput::Pcm(_) => Err(SttError::UnsupportedFormat(
            "PCM input requires WAV-encoding via stt-audio::wav before assembly batch".into(),
        )),
    }
}

fn into_transcript(r: TranscriptResult) -> Transcript {
    let language = r.language_code;
    let duration = r.audio_duration.unwrap_or(0.0);
    let model = r.speech_model;
    let mut segments: Vec<Segment> = Vec::new();
    if let Some(utts) = r.utterances {
        for u in utts {
            let mut words: Vec<Word> = Vec::new();
            if let Some(uw) = u.words {
                for w in uw {
                    words.push(word_from(w));
                }
            }
            segments.push(Segment {
                text: u.text,
                start_ms: u.start,
                end_ms: u.end,
                words,
                speaker: u.speaker.map(|label| SpeakerTag {
                    id: speaker_id_from_label(&label),
                    label: Some(label),
                }),
                confidence: u.confidence,
            });
        }
    } else if let Some(words) = r.words {
        // Build a single segment from word-level data.
        let text = r.text.clone().unwrap_or_default();
        let start_ms = words.first().map(|w| w.start).unwrap_or(0);
        let end_ms = words.last().map(|w| w.end).unwrap_or(start_ms);
        let mapped: Vec<Word> = words.into_iter().map(word_from).collect();
        segments.push(Segment {
            text,
            start_ms,
            end_ms,
            words: mapped,
            speaker: None,
            confidence: None,
        });
    }
    Transcript {
        text: r.text.unwrap_or_default(),
        language,
        segments,
        duration_secs: duration,
        backend: BackendKind::AssemblyAi,
        model_id: model,
        cost_usd: None,
    }
}

fn word_from(w: AssemblyWord) -> Word {
    Word {
        text: w.text,
        start_ms: w.start,
        end_ms: w.end,
        confidence: w.confidence,
    }
}

fn speaker_id_from_label(label: &str) -> u8 {
    // AssemblyAI labels are 'A', 'B', 'C', …
    label
        .chars()
        .next()
        .map(|c| {
            let upper = c.to_ascii_uppercase();
            ((upper as u32).saturating_sub('A' as u32).min(255)) as u8
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_advertise_named_speakers() {
        assert!(matches!(
            CAPS.diarization,
            atomr_agents_stt_core::DiarizationSupport::NamedSpeakers
        ));
        assert!(CAPS.streaming_push);
    }

    #[test]
    fn speaker_id_maps_letters() {
        assert_eq!(speaker_id_from_label("A"), 0);
        assert_eq!(speaker_id_from_label("B"), 1);
        assert_eq!(speaker_id_from_label("D"), 3);
    }
}
