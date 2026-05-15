//! PyO3 bindings for the higher-level voice-session abstractions.
//!
//! This module covers two distinct surfaces from the workspace:
//!
//! - The STT-side `VoiceSession` (turn-detection over a streaming STT
//!   session, exposed as `VoiceMode` / `VoiceEvent` / `VoiceSession`).
//! - The bidirectional `Conversation` from `atomr-agents-tts-voice`
//!   (turn-based or unified-realtime), exposed as
//!   `ConversationMode` / `ConversationOptions` /
//!   `ConversationAgent` / `InboundTranscript` /
//!   `ConversationEvent` / `Conversation`.
//!
//! The `Conversation` surface is built on top of a `TextToSpeech`
//! handle and a `ConversationAgent` that maps user turns to assistant
//! replies. Audio I/O (mic / speaker) lives in separate crates and is
//! wired by callers around this binding.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::Result as AgentResult;
use atomr_agents_stt_core::SttError;
use atomr_agents_stt_voice::{VoiceEvent, VoiceMode, VoiceSession};
use atomr_agents_tts_voice::{
    Conversation, ConversationAgent, ConversationEvent, ConversationMode, ConversationOptions, NoopAgent,
};
use bytes::Bytes;
use parking_lot::Mutex;
use pyo3::exceptions::{PyRuntimeError, PyStopAsyncIteration};
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use tokio::sync::Mutex as AsyncMutex;

use crate::conv::{json_to_py, py_to_json};
use crate::strategy::await_if_coro;
use crate::stt::PyStreamingSession;
use crate::tts::{PyAudioChunk, PyTextToSpeech};

#[pyclass(name = "VoiceMode", module = "atomr_agents._native.voice")]
#[derive(Clone)]
pub struct PyVoiceMode {
    pub(crate) inner: VoiceMode,
}

#[pymethods]
impl PyVoiceMode {
    #[staticmethod]
    fn live() -> Self {
        Self {
            inner: VoiceMode::Live,
        }
    }

    #[staticmethod]
    fn turn_based(silence_ms: u32) -> Self {
        Self {
            inner: VoiceMode::TurnBased { silence_ms },
        }
    }

    #[getter]
    fn kind(&self) -> &'static str {
        match self.inner {
            VoiceMode::Live => "live",
            VoiceMode::TurnBased { .. } => "turn_based",
        }
    }

    #[getter]
    fn silence_ms(&self) -> Option<u32> {
        match self.inner {
            VoiceMode::TurnBased { silence_ms } => Some(silence_ms),
            VoiceMode::Live => None,
        }
    }

    fn __repr__(&self) -> String {
        match self.inner {
            VoiceMode::Live => "VoiceMode.Live".into(),
            VoiceMode::TurnBased { silence_ms } => {
                format!("VoiceMode.TurnBased(silence_ms={silence_ms})")
            }
        }
    }
}

#[pyclass(name = "VoiceEvent", module = "atomr_agents._native.voice")]
#[derive(Clone)]
pub struct PyVoiceEvent {
    pub(crate) inner: VoiceEvent,
}

#[pymethods]
impl PyVoiceEvent {
    #[getter]
    fn kind(&self) -> &'static str {
        match &self.inner {
            VoiceEvent::PartialTranscript(_) => "partial_transcript",
            VoiceEvent::UserTurn { .. } => "user_turn",
            VoiceEvent::SpeakerChange(_) => "speaker_change",
            VoiceEvent::SilenceDetected { .. } => "silence_detected",
            VoiceEvent::InterimWord(_) => "interim_word",
        }
    }

    fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let v = serde_json::to_value(&self.inner).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        json_to_py(py, &v)
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            VoiceEvent::PartialTranscript(t) => format!("VoiceEvent.PartialTranscript({t:?})"),
            VoiceEvent::UserTurn { text, .. } => format!("VoiceEvent.UserTurn({text:?})"),
            VoiceEvent::SpeakerChange(s) => format!("VoiceEvent.SpeakerChange(id={})", s.id),
            VoiceEvent::SilenceDetected { duration_ms } => {
                format!("VoiceEvent.SilenceDetected({duration_ms}ms)")
            }
            VoiceEvent::InterimWord(w) => format!("VoiceEvent.InterimWord({:?})", w.text),
        }
    }
}

#[pyclass(name = "VoiceSession", module = "atomr_agents._native.voice")]
pub struct PyVoiceSession {
    pub(crate) inner: Arc<AsyncMutex<VoiceSession>>,
}

#[pymethods]
impl PyVoiceSession {
    /// `VoiceSession.open(streaming_session, mode)` — wrap an active
    /// `StreamingSession` returned by `SpeechToText.open_stream`.
    /// The streaming session is **consumed** by this call; further
    /// method calls on it return a runtime error.
    #[staticmethod]
    fn open(stream: &Bound<'_, PyStreamingSession>, mode: &PyVoiceMode) -> PyResult<Self> {
        let py_stream = stream.borrow();
        let mode = mode.inner;
        // Run on the shared pyo3-async tokio runtime so VoiceSession's
        // constructor (which calls `tokio::spawn`) has a reactor in
        // context.
        let rt = pyo3_async_runtimes::tokio::get_runtime();
        let session = rt.block_on(async move {
            let raw = py_stream.take().await?;
            Ok::<VoiceSession, pyo3::PyErr>(VoiceSession::open(raw, mode, None))
        })?;
        Ok(Self {
            inner: Arc::new(AsyncMutex::new(session)),
        })
    }

    /// Open an async iterator over `VoiceEvent`s.
    fn events(&self) -> PyVoiceEventIter {
        PyVoiceEventIter {
            session: self.inner.clone(),
        }
    }

    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut g = inner.lock().await;
            g.close()
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))
        })
    }

    fn mode(&self) -> PyResult<PyVoiceMode> {
        let inner = self.inner.clone();
        let rt = pyo3_async_runtimes::tokio::get_runtime();
        let m = rt.block_on(async move {
            let g = inner.lock().await;
            g.mode()
        });
        Ok(PyVoiceMode { inner: m })
    }
}

#[pyclass(name = "VoiceEventIter", module = "atomr_agents._native.voice")]
pub struct PyVoiceEventIter {
    session: Arc<AsyncMutex<VoiceSession>>,
}

#[pymethods]
impl PyVoiceEventIter {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(slf: PyRef<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let session = slf.session.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut g = session.lock().await;
            match g.recv().await {
                Some(Ok(ev)) => Python::with_gil(|py| Py::new(py, PyVoiceEvent { inner: ev })),
                Some(Err(e)) => Err(PyRuntimeError::new_err(e.to_string())),
                None => Err(PyStopAsyncIteration::new_err("")),
            }
        })
    }
}

// =============================================================================
//                   Bidirectional Conversation surface
// =============================================================================

// ----- ConversationMode ------------------------------------------------------

/// `ConversationMode` — `TurnBased` (caller drives user turns via
/// text or pushed audio + a separate STT) or `UnifiedRealtime` (a
/// single backend such as OpenAI Realtime / Gemini Live transcribes
/// inbound audio and emits assistant audio).
#[pyclass(name = "ConversationMode", module = "atomr_agents._native.voice")]
#[derive(Clone)]
pub struct PyConversationMode {
    pub(crate) inner: ConversationMode,
}

#[pymethods]
impl PyConversationMode {
    #[staticmethod]
    fn turn_based() -> Self {
        Self {
            inner: ConversationMode::TurnBased,
        }
    }

    #[staticmethod]
    fn unified_realtime() -> Self {
        Self {
            inner: ConversationMode::UnifiedRealtime,
        }
    }

    #[getter]
    fn kind(&self) -> &'static str {
        match self.inner {
            ConversationMode::TurnBased => "turn_based",
            ConversationMode::UnifiedRealtime => "unified_realtime",
        }
    }

    fn __repr__(&self) -> String {
        format!("ConversationMode.{}", self.kind())
    }
}

// ----- ConversationOptions ---------------------------------------------------

/// Concrete data class mirroring `atomr_agents_tts_voice::ConversationOptions`.
/// All fields are optional; `extra` is a free-form Python object that
/// is JSON-serialised to the underlying `serde_json::Value`.
#[pyclass(name = "ConversationOptions", module = "atomr_agents._native.voice")]
#[derive(Clone, Default)]
pub struct PyConversationOptions {
    pub(crate) inner: ConversationOptions,
}

#[pymethods]
impl PyConversationOptions {
    #[new]
    #[pyo3(signature = (*, voice_id=None, instructions=None, language=None, extra=None))]
    fn new(
        py: Python<'_>,
        voice_id: Option<String>,
        instructions: Option<String>,
        language: Option<String>,
        extra: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let extra_json = match extra {
            Some(e) => Some(py_to_json(py, e)?),
            None => None,
        };
        Ok(Self {
            inner: ConversationOptions {
                voice_id,
                instructions,
                language,
                extra: extra_json,
            },
        })
    }

    #[getter]
    fn voice_id(&self) -> Option<String> {
        self.inner.voice_id.clone()
    }

    #[getter]
    fn instructions(&self) -> Option<String> {
        self.inner.instructions.clone()
    }

    #[getter]
    fn language(&self) -> Option<String> {
        self.inner.language.clone()
    }

    #[getter]
    fn extra(&self, py: Python<'_>) -> PyResult<PyObject> {
        match &self.inner.extra {
            Some(v) => json_to_py(py, v),
            None => Ok(py.None()),
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "ConversationOptions(voice_id={:?}, instructions={:?}, language={:?})",
            self.inner.voice_id, self.inner.instructions, self.inner.language,
        )
    }
}

// ----- ConversationAgent + adapter -------------------------------------------

/// Dyn handle on `Arc<dyn ConversationAgent>`. Construct via
/// `noop_agent()` (echo agent) or `conversation_agent_from_factory(key)`
/// (Python guest registered through `guest.conversation_agent(...)`).
#[pyclass(name = "ConversationAgent", module = "atomr_agents._native.voice")]
#[derive(Clone)]
pub struct PyConversationAgent {
    pub(crate) inner: Arc<dyn ConversationAgent>,
}

#[pymethods]
impl PyConversationAgent {
    fn __repr__(&self) -> String {
        "ConversationAgent(handle)".into()
    }

    /// Drive the agent directly with a finalised user turn. Returns
    /// the assistant's textual reply. Useful for tests; the regular
    /// flow drives this from inside `Conversation::push_text`.
    fn respond<'py>(&self, py: Python<'py>, user_turn: String) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner
                .respond(&user_turn)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))
        })
    }
}

/// Adapts a Python object implementing `respond(user_turn: str) -> str`
/// (sync or async) into a Rust `ConversationAgent`. Mirrors the guest
/// adapter pattern used elsewhere (embedder, tracer, …).
pub(crate) struct PyConversationAgentAdapter {
    pub(crate) target: Arc<PyObject>,
}

#[async_trait]
impl ConversationAgent for PyConversationAgentAdapter {
    async fn respond(&self, user_turn: &str) -> std::result::Result<String, SttError> {
        let target = self.target.clone();
        let turn = user_turn.to_string();
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let instance: Bound<'_, PyAny> = if bound.hasattr("respond")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("respond")?.call1((turn,))?;
            Ok(r.unbind())
        })
        .map_err(|e| SttError::Internal(format!("py conversation agent: {e}")))?;
        // `await_if_coro` funnels Python coroutines through the
        // pyo3-async bridge and returns the resolved value.
        let final_val = adapter_await(coro_or_val)
            .await
            .map_err(|e| SttError::Internal(format!("py conversation agent await: {e}")))?;
        Python::with_gil(|py| final_val.bind(py).extract::<String>())
            .map_err(|e| SttError::Internal(format!("py conversation agent result: {e}")))
    }
}

/// Small wrapper that maps `await_if_coro`'s `AgentError` outcome
/// back into a plain `String` so the `SttError`-returning adapter can
/// `?` on it without dragging `AgentError` into the public surface.
async fn adapter_await(value: PyObject) -> AgentResult<PyObject> {
    await_if_coro(value).await
}

// ----- InboundTranscript -----------------------------------------------------

/// Data class representing a transcribed user utterance.
///
/// The `tts-voice` crate models this as a `ConversationEvent::UserSpoke`
/// (mirroring `tts-core::RealtimeEvent::InboundTranscript`). The
/// binding exposes it as a free-standing value type so Python callers
/// can construct expected transcripts in tests / mocks even when no
/// realtime backend is wired up.
#[pyclass(name = "InboundTranscript", module = "atomr_agents._native.voice")]
#[derive(Clone)]
pub struct PyInboundTranscript {
    #[pyo3(get)]
    pub text: String,
    #[pyo3(get)]
    pub is_final: bool,
}

#[pymethods]
impl PyInboundTranscript {
    #[new]
    #[pyo3(signature = (text, is_final=true))]
    fn new(text: String, is_final: bool) -> Self {
        Self { text, is_final }
    }

    fn __repr__(&self) -> String {
        format!(
            "InboundTranscript(text={:?}, is_final={})",
            self.text, self.is_final
        )
    }
}

// ----- ConversationEvent -----------------------------------------------------

/// Discriminated event emitted from a `Conversation`'s event stream.
/// The variants correspond to `ConversationEvent` in `tts-voice`.
#[pyclass(name = "ConversationEvent", module = "atomr_agents._native.voice")]
#[derive(Clone)]
pub struct PyConversationEvent {
    pub(crate) inner: ConversationEvent,
}

#[pymethods]
impl PyConversationEvent {
    #[getter]
    fn kind(&self) -> &'static str {
        match &self.inner {
            ConversationEvent::UserSpoke { .. } => "user_spoke",
            ConversationEvent::AssistantText { .. } => "assistant_text",
            ConversationEvent::AssistantAudio { .. } => "assistant_audio",
            ConversationEvent::Interrupted => "interrupted",
            ConversationEvent::Done => "done",
        }
    }

    #[getter]
    fn text(&self) -> Option<String> {
        match &self.inner {
            ConversationEvent::UserSpoke { text, .. } | ConversationEvent::AssistantText { text, .. } => {
                Some(text.clone())
            }
            _ => None,
        }
    }

    #[getter]
    fn is_final(&self) -> Option<bool> {
        match &self.inner {
            ConversationEvent::UserSpoke { is_final, .. }
            | ConversationEvent::AssistantText { is_final, .. } => Some(*is_final),
            _ => None,
        }
    }

    /// The decoded `AudioChunk` for `assistant_audio` events; `None`
    /// for every other variant.
    #[getter]
    fn audio(&self) -> Option<PyAudioChunk> {
        match &self.inner {
            ConversationEvent::AssistantAudio { chunk } => Some(PyAudioChunk { inner: chunk.clone() }),
            _ => None,
        }
    }

    fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let v = serde_json::to_value(&self.inner).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        json_to_py(py, &v)
    }

    fn __repr__(&self) -> String {
        format!("ConversationEvent.{}", self.kind())
    }
}

// ----- Factories -------------------------------------------------------------

/// Built-in echo agent that returns `"(echo) {user_turn}"`.
#[pyfunction]
fn noop_agent() -> PyConversationAgent {
    PyConversationAgent {
        inner: Arc::new(NoopAgent),
    }
}

/// Materialise a Python-registered ConversationAgent. The target is
/// looked up under one of the keys
/// `conversation_agent` / `conv_agent` / `agent` for guest convenience.
#[pyfunction]
fn conversation_agent_from_factory(key: String) -> PyResult<PyConversationAgent> {
    let target = crate::guest::must_lookup("conversation_agent", &key)
        .or_else(|_| crate::guest::must_lookup("conv_agent", &key))
        .or_else(|_| crate::guest::must_lookup("agent", &key))?;
    Ok(PyConversationAgent {
        inner: Arc::new(PyConversationAgentAdapter { target }),
    })
}

// ----- Conversation ----------------------------------------------------------

/// Bidirectional voice session built on a `TextToSpeech` handle and a
/// `ConversationAgent`. Construct with one of the static methods:
///
/// - `Conversation.open_turn_based(tts, agent, options=None)` — works
///   with any TTS backend. Caller drives the conversation by
///   `push_text(...)` (typically the finalised output of a separate
///   STT pipeline). The `feed(...)` method is a no-op in this mode.
/// - `Conversation.open_unified_realtime(tts, agent, options=None)` —
///   opens a `RealtimeSession` against a backend such as OpenAI
///   Realtime / Gemini Live / ElevenLabs ConvAI. Caller drives audio
///   in via `feed(...)`; the session forwards transcripts and
///   assistant audio to `events()`.
///
/// A single `Conversation.open(tts, agent, mode, options=None)` shortcut
/// dispatches on the `ConversationMode`.
#[pyclass(name = "Conversation", module = "atomr_agents._native.voice")]
pub struct PyConversation {
    pub(crate) inner: Arc<AsyncMutex<Conversation>>,
    pub(crate) mode: ConversationMode,
}

#[pymethods]
impl PyConversation {
    /// Open a turn-based conversation. Synchronous — no realtime
    /// session is established up front.
    #[staticmethod]
    #[pyo3(signature = (tts, agent, options=None))]
    fn open_turn_based(
        tts: PyTextToSpeech,
        agent: PyConversationAgent,
        options: Option<PyConversationOptions>,
    ) -> PyResult<Self> {
        let opts = options.map(|o| o.inner).unwrap_or_default();
        let conv = Conversation::open_turn_based(tts.inner.clone(), agent.inner.clone(), opts);
        Ok(Self {
            inner: Arc::new(AsyncMutex::new(conv)),
            mode: ConversationMode::TurnBased,
        })
    }

    /// Open a unified-realtime conversation. Async — must be awaited
    /// from Python. Returns a `Conversation` once the backend session
    /// is established.
    #[staticmethod]
    #[pyo3(signature = (tts, agent, options=None))]
    fn open_unified_realtime<'py>(
        py: Python<'py>,
        tts: PyTextToSpeech,
        agent: PyConversationAgent,
        options: Option<PyConversationOptions>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let opts = options.map(|o| o.inner).unwrap_or_default();
        let tts_inner = tts.inner.clone();
        let agent_inner = agent.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let conv = Conversation::open_unified_realtime(tts_inner, agent_inner, opts)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            Python::with_gil(|py| {
                Py::new(
                    py,
                    PyConversation {
                        inner: Arc::new(AsyncMutex::new(conv)),
                        mode: ConversationMode::UnifiedRealtime,
                    },
                )
            })
        })
    }

    /// `Conversation.open(tts, agent, mode, options=None)` — convenience
    /// dispatch over `ConversationMode`. Always async (the unified
    /// realtime branch needs to await session establishment).
    #[staticmethod]
    #[pyo3(signature = (tts, agent, mode, options=None))]
    fn open<'py>(
        py: Python<'py>,
        tts: PyTextToSpeech,
        agent: PyConversationAgent,
        mode: PyConversationMode,
        options: Option<PyConversationOptions>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let opts = options.map(|o| o.inner).unwrap_or_default();
        let tts_inner = tts.inner.clone();
        let agent_inner = agent.inner.clone();
        let mode_inner = mode.inner;
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let conv = match mode_inner {
                ConversationMode::TurnBased => Conversation::open_turn_based(tts_inner, agent_inner, opts),
                ConversationMode::UnifiedRealtime => {
                    Conversation::open_unified_realtime(tts_inner, agent_inner, opts)
                        .await
                        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?
                }
            };
            Python::with_gil(|py| {
                Py::new(
                    py,
                    PyConversation {
                        inner: Arc::new(AsyncMutex::new(conv)),
                        mode: mode_inner,
                    },
                )
            })
        })
    }

    /// Push raw PCM (or container-framed) audio bytes into the
    /// session. Only meaningful in `UnifiedRealtime` mode — in
    /// `TurnBased` mode this resolves immediately as a no-op (the
    /// crate-level `Conversation::push_audio` returns `Ok(())` when
    /// no realtime session is attached). Use `push_text` to drive the
    /// turn-based path.
    fn feed<'py>(&self, py: Python<'py>, pcm_bytes: &Bound<'py, PyBytes>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let bytes = Bytes::copy_from_slice(pcm_bytes.as_bytes());
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut g = inner.lock().await;
            g.push_audio(bytes)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))
        })
    }

    /// Push a finalised user turn into the session. Drives the agent
    /// + TTS pipeline in `TurnBased` mode; forwards as a text utterance
    /// in `UnifiedRealtime` mode.
    fn push_text<'py>(&self, py: Python<'py>, text: String) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut g = inner.lock().await;
            g.push_text(&text)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))
        })
    }

    /// Send a barge-in / interrupt to the backend session.
    fn interrupt<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut g = inner.lock().await;
            g.interrupt()
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))
        })
    }

    /// Close the session. Idempotent — subsequent calls are no-ops.
    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut g = inner.lock().await;
            g.close()
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))
        })
    }

    /// Open an async iterator over `ConversationEvent`s. The underlying
    /// channel is single-consumer: a second call returns an iterator
    /// that yields no events (mirrors the Rust `events()` semantics).
    fn events<'py>(&self, py: Python<'py>) -> PyResult<PyConversationEventStream> {
        let inner = self.inner.clone();
        // Drain the receiver into our own `mpsc` channel via a pumping
        // task — same shape as `PyEventStream` in observability.rs but
        // driven by the borrow-checker-safe `Conversation::events()`.
        let (tx, rx) =
            tokio::sync::mpsc::unbounded_channel::<std::result::Result<ConversationEvent, SttError>>();
        // Spawn a forwarder on the shared tokio runtime so the
        // borrow against `Conversation` (the `events()` Stream's
        // lifetime is `'a` of the session) stays inside the task.
        let rt = pyo3_async_runtimes::tokio::get_runtime();
        let pump = rt.spawn(async move {
            use futures::StreamExt;
            let mut g = inner.lock().await;
            let mut stream = g.events();
            while let Some(item) = stream.next().await {
                if tx.send(item).is_err() {
                    break;
                }
            }
        });
        // We deliberately leak the JoinHandle into the stream wrapper
        // so it lives as long as the iterator. When the iterator is
        // dropped, the receiver hangs up and the pump exits.
        let _ = py; // silence unused-`py` warning
        Ok(PyConversationEventStream {
            rx: Arc::new(AsyncMutex::new(rx)),
            _pump: Arc::new(Mutex::new(Some(pump))),
        })
    }

    /// Current `ConversationMode` (cached at construction).
    #[getter]
    fn mode(&self) -> PyConversationMode {
        PyConversationMode { inner: self.mode }
    }

    fn __repr__(&self) -> String {
        let kind = match self.mode {
            ConversationMode::TurnBased => "turn_based",
            ConversationMode::UnifiedRealtime => "unified_realtime",
        };
        format!("Conversation(mode={kind})")
    }
}

/// Async iterator over `ConversationEvent`s. Mirrors the
/// `PyEventStream` pattern in `observability.rs`.
#[pyclass(name = "ConversationEventStream", module = "atomr_agents._native.voice")]
pub struct PyConversationEventStream {
    rx: Arc<
        AsyncMutex<tokio::sync::mpsc::UnboundedReceiver<std::result::Result<ConversationEvent, SttError>>>,
    >,
    // Keep the pump task alive for the lifetime of the stream. Wrapped
    // in an `Arc<Mutex<Option<_>>>` so the iterator stays `Send`.
    _pump: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

#[pymethods]
impl PyConversationEventStream {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(slf: PyRef<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let rx = slf.rx.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = rx.lock().await;
            match guard.recv().await {
                Some(Ok(ev)) => Python::with_gil(|py| Py::new(py, PyConversationEvent { inner: ev })),
                Some(Err(e)) => Err(PyRuntimeError::new_err(e.to_string())),
                None => Err(PyStopAsyncIteration::new_err("")),
            }
        })
    }
}

// =============================================================================
//                           Module registration
// =============================================================================

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "voice")?;
    // STT-side surface.
    m.add_class::<PyVoiceMode>()?;
    m.add_class::<PyVoiceEvent>()?;
    m.add_class::<PyVoiceSession>()?;
    m.add_class::<PyVoiceEventIter>()?;
    // Bidirectional Conversation surface.
    m.add_class::<PyConversationMode>()?;
    m.add_class::<PyConversationOptions>()?;
    m.add_class::<PyConversationAgent>()?;
    m.add_class::<PyInboundTranscript>()?;
    m.add_class::<PyConversationEvent>()?;
    m.add_class::<PyConversation>()?;
    m.add_class::<PyConversationEventStream>()?;
    m.add_function(wrap_pyfunction!(noop_agent, &m)?)?;
    m.add_function(wrap_pyfunction!(conversation_agent_from_factory, &m)?)?;
    parent.add_submodule(&m)?;
    Ok(())
}
