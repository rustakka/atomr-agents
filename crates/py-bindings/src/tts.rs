//! PyO3 bindings for the text-to-speech capability.
//!
//! Step-1 surface: `Capabilities` (round-tripped to a Python dict
//! via serde_json), `VoiceRef` factories, `SynthesisRequest`
//! factories, `AudioOutput`, `TextToSpeech`, `SynthesisStream`
//! (async iterator), `RealtimeSession` (async iterator), plus a
//! `mock_tts()` constructor that exercises the full FFI shape
//! without any backend.
//!
//! Backend constructors (`tts_openai`, `tts_elevenlabs`, …) are
//! added later in step 14 after the runtime crates land.

use std::sync::Arc;

use atomr_agents_tts_core::{
    AudioInput, AudioOutput, BackendKind, Capabilities, DialogueTurn, DynTextToSpeech,
    MockTextToSpeech, RealtimeEvent, RealtimeOptions, RealtimeSession, SpeakerVoice,
    SynthOptions, SynthesisRequest, SynthesisStream, VoiceRef,
};
use bytes::Bytes;
use futures::StreamExt;
use pyo3::exceptions::{PyRuntimeError, PyStopAsyncIteration, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use tokio::sync::Mutex as AsyncMutex;

use crate::conv::{json_to_py, py_to_json};
use crate::stt::PyAudioInput;

// ----- Capabilities ---------------------------------------------------------

#[pyclass(name = "Capabilities", module = "atomr_agents._native.tts")]
#[derive(Clone)]
pub struct PyCapabilities {
    pub(crate) inner: Capabilities,
}

#[pymethods]
impl PyCapabilities {
    fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let v = serde_json::to_value(&self.inner)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        json_to_py(py, &v)
    }

    #[getter]
    fn plain_tts(&self) -> bool { self.inner.plain_tts }
    #[getter]
    fn voicegen_from_text(&self) -> bool { self.inner.voicegen_from_text }
    #[getter]
    fn voice_cloning(&self, py: Python<'_>) -> PyResult<PyObject> {
        let v = serde_json::to_value(&self.inner.voice_cloning)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        json_to_py(py, &v)
    }
    #[getter]
    fn dialogue_multispeaker(&self) -> Option<u8> { self.inner.dialogue_multispeaker }
    #[getter]
    fn sound_effects(&self) -> bool { self.inner.sound_effects }
    #[getter]
    fn realtime_bidirectional(&self) -> bool { self.inner.realtime_bidirectional }
    #[getter]
    fn streaming_output(&self) -> bool { self.inner.streaming_output }
    #[getter]
    fn style_control(&self) -> bool { self.inner.style_control }
    #[getter]
    fn ssml(&self) -> bool { self.inner.ssml }
    #[getter]
    fn prosody_control(&self) -> bool { self.inner.prosody_control }
    #[getter]
    fn word_timestamps(&self) -> bool { self.inner.word_timestamps }
    #[getter]
    fn requires_network(&self) -> bool { self.inner.requires_network }
    #[getter]
    fn typical_ttfb_ms(&self) -> Option<u16> { self.inner.typical_ttfb_ms }
    #[getter]
    fn cost_per_1k_chars_usd(&self) -> Option<f32> { self.inner.cost_per_1k_chars_usd }
    #[getter]
    fn cost_per_audio_min_usd(&self) -> Option<f32> { self.inner.cost_per_audio_min_usd }
    #[getter]
    fn max_chars_per_request(&self) -> Option<u32> { self.inner.max_chars_per_request }

    fn __repr__(&self) -> String {
        format!(
            "Capabilities(plain_tts={}, voicegen={}, dialogue={:?}, sfx={}, realtime={})",
            self.inner.plain_tts,
            self.inner.voicegen_from_text,
            self.inner.dialogue_multispeaker,
            self.inner.sound_effects,
            self.inner.realtime_bidirectional,
        )
    }
}

// ----- VoiceRef -------------------------------------------------------------

#[pyclass(name = "VoiceRef", module = "atomr_agents._native.tts")]
#[derive(Clone)]
pub struct PyVoiceRef {
    pub(crate) inner: VoiceRef,
}

#[pyfunction]
pub fn voice_library(id: String) -> PyVoiceRef {
    PyVoiceRef { inner: VoiceRef::library(id) }
}

#[pyfunction]
pub fn voice_described(description: String) -> PyVoiceRef {
    PyVoiceRef { inner: VoiceRef::described(description) }
}

#[pyfunction]
pub fn voice_cloned(audio: PyAudioInput) -> PyVoiceRef {
    PyVoiceRef { inner: VoiceRef::cloned_from(audio.inner) }
}

// ----- SynthesisRequest -----------------------------------------------------

#[pyclass(name = "SynthesisRequest", module = "atomr_agents._native.tts")]
#[derive(Clone)]
pub struct PySynthesisRequest {
    pub(crate) inner: SynthesisRequest,
}

fn opts_from_kwargs(
    py: Python<'_>,
    language: Option<String>,
    model: Option<String>,
    style: Option<String>,
    pitch: Option<f32>,
    rate: Option<f32>,
    volume: Option<f32>,
    extra: Option<&Bound<'_, PyAny>>,
) -> PyResult<SynthOptions> {
    let extra_json = match extra {
        Some(e) => Some(py_to_json(py, e)?),
        None => None,
    };
    Ok(SynthOptions {
        language,
        model,
        style,
        pitch,
        rate,
        volume,
        format: None,
        extra: extra_json,
    })
}

/// `tts_request(text, voice, *, language=None, model=None, style=None, pitch=None, rate=None, volume=None, extra=None)`.
#[pyfunction]
#[pyo3(signature = (
    text, voice, *, language=None, model=None, style=None, pitch=None,
    rate=None, volume=None, extra=None
))]
#[allow(clippy::too_many_arguments)]
pub fn tts_request(
    py: Python<'_>,
    text: String,
    voice: PyVoiceRef,
    language: Option<String>,
    model: Option<String>,
    style: Option<String>,
    pitch: Option<f32>,
    rate: Option<f32>,
    volume: Option<f32>,
    extra: Option<&Bound<'_, PyAny>>,
) -> PyResult<PySynthesisRequest> {
    let options = opts_from_kwargs(py, language, model, style, pitch, rate, volume, extra)?;
    Ok(PySynthesisRequest {
        inner: SynthesisRequest::Tts {
            text,
            voice: voice.inner,
            options,
        },
    })
}

#[pyfunction]
#[pyo3(signature = (prompt, *, duration_secs=None, extra=None))]
pub fn sfx_request(
    py: Python<'_>,
    prompt: String,
    duration_secs: Option<f32>,
    extra: Option<&Bound<'_, PyAny>>,
) -> PyResult<PySynthesisRequest> {
    let options = opts_from_kwargs(py, None, None, None, None, None, None, extra)?;
    Ok(PySynthesisRequest {
        inner: SynthesisRequest::SoundEffect {
            prompt,
            duration_secs,
            options,
        },
    })
}

/// `dialogue_request(script, speakers)` — `script` is a list of
/// `(speaker_tag, text)` tuples; `speakers` is a list of `(tag, voice)`
/// tuples.
#[pyfunction]
pub fn dialogue_request(
    script: Vec<(String, String)>,
    speakers: Vec<(String, PyVoiceRef)>,
) -> PyResult<PySynthesisRequest> {
    let script: Vec<DialogueTurn> = script
        .into_iter()
        .map(|(speaker, text)| DialogueTurn { speaker, text })
        .collect();
    let speakers: Vec<SpeakerVoice> = speakers
        .into_iter()
        .map(|(tag, voice)| SpeakerVoice { tag, voice: voice.inner })
        .collect();
    Ok(PySynthesisRequest {
        inner: SynthesisRequest::Dialogue {
            script,
            speakers,
            options: SynthOptions::default(),
        },
    })
}

// ----- AudioOutput ----------------------------------------------------------

#[pyclass(name = "AudioOutput", module = "atomr_agents._native.tts")]
#[derive(Clone)]
pub struct PyAudioOutput {
    pub(crate) inner: AudioOutput,
}

#[pymethods]
impl PyAudioOutput {
    #[getter]
    fn duration_secs(&self) -> f32 { self.inner.duration_secs }
    #[getter]
    fn characters_processed(&self) -> u32 { self.inner.characters_processed }
    #[getter]
    fn backend(&self) -> String { self.inner.backend.as_str().to_string() }
    #[getter]
    fn model_id(&self) -> Option<String> { self.inner.model_id.clone() }
    #[getter]
    fn voice_id_used(&self) -> Option<String> { self.inner.voice_id_used.clone() }
    #[getter]
    fn cost_usd(&self) -> Option<f32> { self.inner.cost_usd }
    #[getter]
    fn sample_rate(&self) -> u32 { self.inner.audio.sample_rate }
    #[getter]
    fn channels(&self) -> u16 { self.inner.audio.channels }

    /// Return the raw PCM samples as a flat list of f32.
    fn samples(&self) -> Vec<f32> {
        self.inner.audio.samples.clone()
    }

    /// Return the container bytes (when the backend emitted MP3 /
    /// Opus / etc directly), or `None` if the result is PCM-only.
    fn container_bytes<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.inner
            .container_bytes
            .as_ref()
            .map(|b| PyBytes::new_bound(py, b.as_ref()))
    }

    fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let v = serde_json::to_value(&self.inner)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        json_to_py(py, &v)
    }

    fn __repr__(&self) -> String {
        format!(
            "AudioOutput(backend={:?}, secs={:.2}, voice={:?})",
            self.inner.backend.as_str(),
            self.inner.duration_secs,
            self.inner.voice_id_used,
        )
    }
}

// ----- AudioChunk + SynthesisStream -----------------------------------------

#[pyclass(name = "AudioChunk", module = "atomr_agents._native.tts")]
#[derive(Clone)]
pub struct PyAudioChunk {
    pub(crate) inner: atomr_agents_tts_core::AudioChunk,
}

#[pymethods]
impl PyAudioChunk {
    #[getter]
    fn seq(&self) -> u64 { self.inner.seq }
    #[getter]
    fn is_final(&self) -> bool { self.inner.is_final }

    fn bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, self.inner.bytes.as_ref())
    }

    fn __repr__(&self) -> String {
        format!(
            "AudioChunk(seq={}, bytes={}, is_final={})",
            self.inner.seq,
            self.inner.bytes.len(),
            self.inner.is_final,
        )
    }
}

#[pyclass(name = "SynthesisStream", module = "atomr_agents._native.tts")]
pub struct PySynthesisStream {
    pub(crate) inner: Arc<AsyncMutex<Option<Box<dyn SynthesisStream>>>>,
}

impl PySynthesisStream {
    pub(crate) fn new(s: Box<dyn SynthesisStream>) -> Self {
        Self {
            inner: Arc::new(AsyncMutex::new(Some(s))),
        }
    }
}

fn consumed_err() -> pyo3::PyErr {
    PyRuntimeError::new_err("SynthesisStream was consumed or closed")
}

#[pymethods]
impl PySynthesisStream {
    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut g = inner.lock().await;
            if let Some(s) = g.as_mut() {
                s.close().await.map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            }
            Ok(())
        })
    }

    fn events(&self) -> PyAudioChunkIter {
        PyAudioChunkIter {
            session: self.inner.clone(),
        }
    }
}

#[pyclass(name = "AudioChunkIter", module = "atomr_agents._native.tts")]
pub struct PyAudioChunkIter {
    session: Arc<AsyncMutex<Option<Box<dyn SynthesisStream>>>>,
}

#[pymethods]
impl PyAudioChunkIter {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> { slf }

    fn __anext__<'py>(slf: PyRef<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let session = slf.session.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = session.lock().await;
            let s = guard.as_mut().ok_or_else(consumed_err)?;
            let mut stream = s.events();
            match stream.next().await {
                Some(Ok(c)) => {
                    drop(stream);
                    Python::with_gil(|py| Py::new(py, PyAudioChunk { inner: c }))
                }
                Some(Err(e)) => Err(PyRuntimeError::new_err(e.to_string())),
                None => Err(PyStopAsyncIteration::new_err("")),
            }
        })
    }
}

// ----- RealtimeSession -------------------------------------------------------

#[pyclass(name = "RealtimeEvent", module = "atomr_agents._native.tts")]
#[derive(Clone)]
pub struct PyRealtimeEvent {
    pub(crate) inner: RealtimeEvent,
}

#[pymethods]
impl PyRealtimeEvent {
    #[getter]
    fn kind(&self) -> &'static str {
        match &self.inner {
            RealtimeEvent::AudioOut { .. } => "audio_out",
            RealtimeEvent::InboundTranscript { .. } => "inbound_transcript",
            RealtimeEvent::OutboundText { .. } => "outbound_text",
            RealtimeEvent::OutboundWords { .. } => "outbound_words",
            RealtimeEvent::UserSpeechStarted => "user_speech_started",
            RealtimeEvent::UserSpeechEnded => "user_speech_ended",
            RealtimeEvent::BargeIn => "barge_in",
            RealtimeEvent::Done => "done",
            RealtimeEvent::Metadata(_) => "metadata",
        }
    }

    fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let v = serde_json::to_value(&self.inner)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        json_to_py(py, &v)
    }

    fn __repr__(&self) -> String {
        format!("RealtimeEvent.{}", self.kind())
    }
}

#[pyclass(name = "RealtimeSession", module = "atomr_agents._native.tts")]
pub struct PyRealtimeSession {
    pub(crate) inner: Arc<AsyncMutex<Option<Box<dyn RealtimeSession>>>>,
}

impl PyRealtimeSession {
    pub(crate) fn new(s: Box<dyn RealtimeSession>) -> Self {
        Self {
            inner: Arc::new(AsyncMutex::new(Some(s))),
        }
    }

    pub(crate) async fn take(&self) -> PyResult<Box<dyn RealtimeSession>> {
        let mut g = self.inner.lock().await;
        g.take().ok_or_else(|| {
            PyRuntimeError::new_err("RealtimeSession already consumed")
        })
    }
}

fn rt_consumed_err() -> pyo3::PyErr {
    PyRuntimeError::new_err("RealtimeSession was consumed or closed")
}

#[pymethods]
impl PyRealtimeSession {
    fn push_text<'py>(
        &self,
        py: Python<'py>,
        text: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut g = inner.lock().await;
            let s = g.as_mut().ok_or_else(rt_consumed_err)?;
            s.push_text(&text)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))
        })
    }

    fn push_audio<'py>(
        &self,
        py: Python<'py>,
        data: &Bound<'py, PyBytes>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let bytes = Bytes::copy_from_slice(data.as_bytes());
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut g = inner.lock().await;
            let s = g.as_mut().ok_or_else(rt_consumed_err)?;
            s.push_audio(bytes)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))
        })
    }

    fn commit_input<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut g = inner.lock().await;
            let s = g.as_mut().ok_or_else(rt_consumed_err)?;
            s.commit_input()
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))
        })
    }

    fn interrupt<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut g = inner.lock().await;
            let s = g.as_mut().ok_or_else(rt_consumed_err)?;
            s.interrupt()
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))
        })
    }

    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut g = inner.lock().await;
            if let Some(s) = g.as_mut() {
                s.close().await.map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            }
            Ok(())
        })
    }

    fn events(&self) -> PyRealtimeEventIter {
        PyRealtimeEventIter {
            session: self.inner.clone(),
        }
    }
}

#[pyclass(name = "RealtimeEventIter", module = "atomr_agents._native.tts")]
pub struct PyRealtimeEventIter {
    session: Arc<AsyncMutex<Option<Box<dyn RealtimeSession>>>>,
}

#[pymethods]
impl PyRealtimeEventIter {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> { slf }

    fn __anext__<'py>(slf: PyRef<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let session = slf.session.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = session.lock().await;
            let s = guard.as_mut().ok_or_else(rt_consumed_err)?;
            let mut stream = s.events();
            match stream.next().await {
                Some(Ok(ev)) => {
                    drop(stream);
                    Python::with_gil(|py| Py::new(py, PyRealtimeEvent { inner: ev }))
                }
                Some(Err(e)) => Err(PyRuntimeError::new_err(e.to_string())),
                None => Err(PyStopAsyncIteration::new_err("")),
            }
        })
    }
}

// ----- TextToSpeech (instance) ----------------------------------------------

#[pyclass(name = "TextToSpeech", module = "atomr_agents._native.tts")]
#[derive(Clone)]
pub struct PyTextToSpeech {
    pub(crate) inner: DynTextToSpeech,
}

#[pymethods]
impl PyTextToSpeech {
    fn capabilities(&self) -> PyCapabilities {
        PyCapabilities {
            inner: self.inner.capabilities().clone(),
        }
    }

    fn backend_kind(&self) -> String {
        self.inner.backend_kind().as_str().to_string()
    }

    fn transport_kind(&self) -> String {
        self.inner.transport_kind().as_str().to_string()
    }

    fn synthesize<'py>(
        &self,
        py: Python<'py>,
        request: PySynthesisRequest,
    ) -> PyResult<Bound<'py, PyAny>> {
        let tts = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let out = tts
                .synthesize(request.inner)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            Python::with_gil(|py| Py::new(py, PyAudioOutput { inner: out }))
        })
    }

    fn synthesize_stream<'py>(
        &self,
        py: Python<'py>,
        request: PySynthesisRequest,
    ) -> PyResult<Bound<'py, PyAny>> {
        let tts = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let s = tts
                .synthesize_stream(request.inner)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            Python::with_gil(|py| Py::new(py, PySynthesisStream::new(s)))
        })
    }

    #[pyo3(signature = (
        *, voice_id=None, instructions=None, language=None,
        temperature=None, extra=None
    ))]
    fn open_realtime<'py>(
        &self,
        py: Python<'py>,
        voice_id: Option<String>,
        instructions: Option<String>,
        language: Option<String>,
        temperature: Option<f32>,
        extra: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let opts = RealtimeOptions {
            voice_id,
            instructions,
            language,
            temperature,
            extra: extra.map(|e| py_to_json(py, e)).transpose()?,
        };
        let tts = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let s = tts
                .open_realtime(opts)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            Python::with_gil(|py| Py::new(py, PyRealtimeSession::new(s)))
        })
    }
}

// ----- Free constructors ----------------------------------------------------

#[pyfunction]
pub fn mock_tts() -> PyTextToSpeech {
    PyTextToSpeech {
        inner: Arc::new(MockTextToSpeech::new()),
    }
}

fn tts_secret_ref(api_key: &str) -> atomr_agents_stt_remote_core::SecretRef {
    if let Some(env_name) = api_key.strip_prefix("env:") {
        atomr_agents_stt_remote_core::SecretRef::env(env_name)
    } else if let Some(file_path) = api_key.strip_prefix("file:") {
        atomr_agents_stt_remote_core::SecretRef::file(std::path::PathBuf::from(file_path))
    } else {
        atomr_agents_stt_remote_core::SecretRef::literal(api_key)
    }
}

/// `tts_openai(api_key, *, model=None, voice=None, response_format=None)`.
/// `api_key` accepts a literal key, `"env:VARNAME"`, or `"file:/path"`.
#[pyfunction]
#[pyo3(signature = (api_key, *, model=None, voice=None, response_format=None))]
pub fn tts_openai(
    api_key: &str,
    model: Option<String>,
    voice: Option<String>,
    response_format: Option<String>,
) -> PyResult<PyTextToSpeech> {
    let mut cfg = atomr_agents_tts_runtime_openai::OpenAiTtsConfig::from_env();
    cfg.api_key = tts_secret_ref(api_key);
    if let Some(m) = model { cfg.default_model = m; }
    if let Some(v) = voice { cfg.default_voice = v; }
    if let Some(f) = response_format { cfg.default_format = f; }
    let runner = atomr_agents_tts_runtime_openai::OpenAiTtsRunner::new(cfg)
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    Ok(PyTextToSpeech { inner: Arc::new(runner) })
}

/// `tts_elevenlabs(api_key, *, model=None, voice=None, output_format=None, agent_id=None)`.
#[pyfunction]
#[pyo3(signature = (api_key, *, model=None, voice=None, output_format=None, agent_id=None))]
pub fn tts_elevenlabs(
    api_key: &str,
    model: Option<String>,
    voice: Option<String>,
    output_format: Option<String>,
    agent_id: Option<String>,
) -> PyResult<PyTextToSpeech> {
    let mut cfg = atomr_agents_tts_runtime_elevenlabs::ElevenLabsConfig::from_env();
    cfg.api_key = tts_secret_ref(api_key);
    if let Some(m) = model { cfg.default_model = m; }
    if let Some(v) = voice { cfg.default_voice = v; }
    if let Some(f) = output_format { cfg.default_output_format = f; }
    if agent_id.is_some() { cfg.convai_agent_id = agent_id; }
    let runner = atomr_agents_tts_runtime_elevenlabs::ElevenLabsRunner::new(cfg)
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    Ok(PyTextToSpeech { inner: Arc::new(runner) })
}

/// `tts_openai_realtime(api_key, *, model=None, voice=None, instructions=None)`.
#[pyfunction]
#[pyo3(signature = (api_key, *, model=None, voice=None, instructions=None))]
pub fn tts_openai_realtime(
    api_key: &str,
    model: Option<String>,
    voice: Option<String>,
    instructions: Option<String>,
) -> PyResult<PyTextToSpeech> {
    let mut cfg = atomr_agents_tts_runtime_openai_realtime::OpenAiRealtimeConfig::from_env();
    cfg.api_key = tts_secret_ref(api_key);
    if let Some(m) = model { cfg.model = m; }
    if let Some(v) = voice { cfg.default_voice = v; }
    if instructions.is_some() { cfg.instructions = instructions; }
    let runner = atomr_agents_tts_runtime_openai_realtime::OpenAiRealtimeRunner::new(cfg);
    Ok(PyTextToSpeech { inner: Arc::new(runner) })
}

/// `tts_gemini_live(api_key, *, model=None, voice=None, instructions=None)`.
#[pyfunction]
#[pyo3(signature = (api_key, *, model=None, voice=None, instructions=None))]
pub fn tts_gemini_live(
    api_key: &str,
    model: Option<String>,
    voice: Option<String>,
    instructions: Option<String>,
) -> PyResult<PyTextToSpeech> {
    let mut cfg = atomr_agents_tts_runtime_gemini_live::GeminiLiveConfig::from_env();
    cfg.api_key = tts_secret_ref(api_key);
    if let Some(m) = model { cfg.model = m; }
    if let Some(v) = voice { cfg.default_voice = v; }
    if instructions.is_some() { cfg.instructions = instructions; }
    let runner = atomr_agents_tts_runtime_gemini_live::GeminiLiveRunner::new(cfg);
    Ok(PyTextToSpeech { inner: Arc::new(runner) })
}

/// `tts_piper(*, voices=None)`. `voices` is a list of dicts:
/// `{"id": str, "onnx_path": str, "config_path": str, "language": str|None}`.
/// Returns a runner that errors on synthesize unless built with `--features tts-piper-ort`.
#[pyfunction]
#[pyo3(signature = (voices=None, *, length_scale=None, noise_scale=None, noise_w=None, use_gpu=false))]
pub fn tts_piper(
    voices: Option<Vec<Bound<'_, pyo3::types::PyDict>>>,
    length_scale: Option<f32>,
    noise_scale: Option<f32>,
    noise_w: Option<f32>,
    use_gpu: bool,
) -> PyResult<PyTextToSpeech> {
    let mut cfg = atomr_agents_tts_runtime_piper::PiperConfig::default();
    if let Some(list) = voices {
        for d in list {
            let id: String = d.get_item("id")?.unwrap().extract()?;
            let onnx: String = d.get_item("onnx_path")?.unwrap().extract()?;
            let conf: String = d.get_item("config_path")?.unwrap().extract()?;
            let lang: Option<String> = match d.get_item("language")? {
                Some(v) if !v.is_none() => Some(v.extract()?),
                _ => None,
            };
            cfg.voices.push(atomr_agents_tts_runtime_piper::PiperVoiceModel {
                id,
                onnx_path: std::path::PathBuf::from(onnx),
                config_path: std::path::PathBuf::from(conf),
                language: lang,
            });
        }
    }
    if let Some(v) = length_scale { cfg.length_scale = v; }
    if let Some(v) = noise_scale { cfg.noise_scale = v; }
    if let Some(v) = noise_w { cfg.noise_w = v; }
    cfg.use_gpu = use_gpu;
    let runner = atomr_agents_tts_runtime_piper::PiperRunner::new(cfg)
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    Ok(PyTextToSpeech { inner: Arc::new(runner) })
}

/// `tts_kokoro(model_path, voices_path, *, default_voice=None, speed=None, use_gpu=False)`.
#[pyfunction]
#[pyo3(signature = (model_path, voices_path, *, default_voice=None, speed=None, use_gpu=false))]
pub fn tts_kokoro(
    model_path: String,
    voices_path: String,
    default_voice: Option<String>,
    speed: Option<f32>,
    use_gpu: bool,
) -> PyResult<PyTextToSpeech> {
    let mut cfg = atomr_agents_tts_runtime_kokoro::KokoroConfig::default();
    cfg.model_path = std::path::PathBuf::from(model_path);
    cfg.voices_path = std::path::PathBuf::from(voices_path);
    if let Some(v) = default_voice { cfg.default_voice = v; }
    if let Some(s) = speed { cfg.speed = s; }
    cfg.use_gpu = use_gpu;
    let runner = atomr_agents_tts_runtime_kokoro::KokoroRunner::new(cfg)
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    Ok(PyTextToSpeech { inner: Arc::new(runner) })
}

/// `tts_moss(*, endpoint=None, model_variant=None, default_voice=None, bearer_token=None)`.
#[pyfunction]
#[pyo3(signature = (*, endpoint=None, model_variant=None, default_voice=None, bearer_token=None))]
pub fn tts_moss(
    endpoint: Option<String>,
    model_variant: Option<String>,
    default_voice: Option<String>,
    bearer_token: Option<String>,
) -> PyResult<PyTextToSpeech> {
    let mut cfg = atomr_agents_tts_runtime_moss::MossTtsConfig::default();
    if let Some(e) = endpoint {
        cfg.endpoint = url::Url::parse(&e)
            .map_err(|err| PyValueError::new_err(format!("bad endpoint: {err}")))?;
    }
    if let Some(v) = model_variant {
        cfg.model_variant = match v.as_str() {
            "delay_8b" => atomr_agents_tts_runtime_moss::MossModelVariant::Delay8B,
            "local_1_7b" => atomr_agents_tts_runtime_moss::MossModelVariant::Local1_7B,
            "tssd" => atomr_agents_tts_runtime_moss::MossModelVariant::Tssd,
            "voice_generator" => atomr_agents_tts_runtime_moss::MossModelVariant::VoiceGenerator,
            "sound_effect" => atomr_agents_tts_runtime_moss::MossModelVariant::SoundEffect,
            "realtime" => atomr_agents_tts_runtime_moss::MossModelVariant::Realtime,
            other => return Err(PyValueError::new_err(format!("unknown model_variant: {other}"))),
        };
    }
    if default_voice.is_some() { cfg.default_voice = default_voice; }
    if bearer_token.is_some() { cfg.bearer_token = bearer_token; }
    let runner = atomr_agents_tts_runtime_moss::MossTtsRunner::new(cfg)
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    Ok(PyTextToSpeech { inner: Arc::new(runner) })
}

/// `tts_xtts(*, endpoint=None, default_speaker=None, default_language=None, bearer_token=None)`.
#[pyfunction]
#[pyo3(signature = (*, endpoint=None, default_speaker=None, default_language=None, bearer_token=None))]
pub fn tts_xtts(
    endpoint: Option<String>,
    default_speaker: Option<String>,
    default_language: Option<String>,
    bearer_token: Option<String>,
) -> PyResult<PyTextToSpeech> {
    let mut cfg = atomr_agents_tts_runtime_xtts::XttsConfig::default();
    if let Some(e) = endpoint {
        cfg.endpoint = url::Url::parse(&e)
            .map_err(|err| PyValueError::new_err(format!("bad endpoint: {err}")))?;
    }
    if default_speaker.is_some() { cfg.default_speaker = default_speaker; }
    if let Some(l) = default_language { cfg.default_language = l; }
    if bearer_token.is_some() { cfg.bearer_token = bearer_token; }
    let runner = atomr_agents_tts_runtime_xtts::XttsRunner::new(cfg)
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    Ok(PyTextToSpeech { inner: Arc::new(runner) })
}

// Silence unused for items we keep for future steps.
#[allow(dead_code)]
fn _unused_marker(_a: AudioInput, _b: BackendKind, _v: PyValueError) {}

// ----- Module registration --------------------------------------------------

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "tts")?;
    m.add_class::<PyCapabilities>()?;
    m.add_class::<PyVoiceRef>()?;
    m.add_class::<PySynthesisRequest>()?;
    m.add_class::<PyAudioOutput>()?;
    m.add_class::<PyAudioChunk>()?;
    m.add_class::<PySynthesisStream>()?;
    m.add_class::<PyAudioChunkIter>()?;
    m.add_class::<PyRealtimeEvent>()?;
    m.add_class::<PyRealtimeSession>()?;
    m.add_class::<PyRealtimeEventIter>()?;
    m.add_class::<PyTextToSpeech>()?;
    m.add_function(wrap_pyfunction!(voice_library, &m)?)?;
    m.add_function(wrap_pyfunction!(voice_described, &m)?)?;
    m.add_function(wrap_pyfunction!(voice_cloned, &m)?)?;
    m.add_function(wrap_pyfunction!(tts_request, &m)?)?;
    m.add_function(wrap_pyfunction!(sfx_request, &m)?)?;
    m.add_function(wrap_pyfunction!(dialogue_request, &m)?)?;
    m.add_function(wrap_pyfunction!(mock_tts, &m)?)?;
    m.add_function(wrap_pyfunction!(tts_openai, &m)?)?;
    m.add_function(wrap_pyfunction!(tts_elevenlabs, &m)?)?;
    m.add_function(wrap_pyfunction!(tts_openai_realtime, &m)?)?;
    m.add_function(wrap_pyfunction!(tts_gemini_live, &m)?)?;
    m.add_function(wrap_pyfunction!(tts_piper, &m)?)?;
    m.add_function(wrap_pyfunction!(tts_kokoro, &m)?)?;
    m.add_function(wrap_pyfunction!(tts_moss, &m)?)?;
    m.add_function(wrap_pyfunction!(tts_xtts, &m)?)?;
    parent.add_submodule(&m)?;
    Ok(())
}
