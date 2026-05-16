//! Python bindings for the avatar capability.
//!
//! Exposes `atomr_agents._native.avatar`:
//!
//! - `AvatarHarness` — top-level orchestrator. Accepts a Python-side
//!   inference callable (driven through atomr-infer downstream), a
//!   `TextToSpeech` produced by the existing tts bindings, and a
//!   sink (LiveLink UDP on x86_64 or the in-memory capture sink for
//!   tests). Async methods return awaitables.
//! - `AvatarFrame` — read-only view of the frames the harness emits.
//! - `LiveLinkSink` / `CapturingSink` — sink factories (the latter is
//!   always present for tests; the former requires the
//!   `avatar-livelink` cargo feature and x86_64).
//!
//! Per-module note: this is gated by the `avatar` cargo feature in
//! [`crate::lib`] so aarch64 wheels can omit it entirely.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_avatar_core::{
    AvatarError, AvatarFrame, AvatarSink, BlendshapeWeights, EmotionVector, Result, SinkKind,
    SmpteTimecode,
};
use atomr_agents_avatar_harness::{
    AgentIntentPacket, AvatarHarness, AvatarHarnessBuilder, AvatarHarnessConfig,
    AvatarInferenceClient, CognitionConfig, SyncConfig,
};
use atomr_agents_tts_core::{DynTextToSpeech, VoiceRef};
use atomr_infer_core::batch::ExecuteBatch;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict};

use crate::tts::PyTextToSpeech;

fn rt_err(e: impl std::fmt::Display) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}

fn val_err(e: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(e.to_string())
}

// ----- AvatarFrame (read-only Python view) ---------------------------------

#[pyclass(name = "AvatarFrame", module = "atomr_agents._native.avatar")]
#[derive(Clone)]
pub struct PyAvatarFrame {
    pub(crate) inner: AvatarFrame,
}

#[pymethods]
impl PyAvatarFrame {
    /// SMPTE timecode formatted as `HH:MM:SS:FF`.
    fn timecode(&self) -> String {
        self.inner.timecode.format()
    }

    /// Frame rate the timecode was generated against.
    #[getter]
    fn frame_rate(&self) -> u8 {
        self.inner.timecode.frame_rate
    }

    /// Sample rate of the embedded audio chunk, in Hz.
    #[getter]
    fn sample_rate_hz(&self) -> u32 {
        self.inner.audio.sample_rate_hz
    }

    /// Raw audio bytes (s16-LE).
    fn audio_bytes(&self) -> &[u8] {
        &self.inner.audio.samples_s16le
    }

    /// 52-element list of ARKit blendshape weights.
    fn weights(&self) -> Vec<f32> {
        self.inner.weights.as_array().to_vec()
    }

    /// Optional emotion snapshot at this tick — `None` if the
    /// harness has emotion overlays disabled.
    fn emotion<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyDict>> {
        self.inner.emotion.map(|e| emotion_to_dict(py, &e))
    }

    fn __repr__(&self) -> String {
        format!(
            "AvatarFrame(timecode={}, sample_rate={}Hz, audio_bytes={}, weights_len=52)",
            self.inner.timecode.format(),
            self.inner.audio.sample_rate_hz,
            self.inner.audio.samples_s16le.len(),
        )
    }
}

fn emotion_to_dict<'py>(py: Python<'py>, e: &EmotionVector) -> Bound<'py, PyDict> {
    let d = PyDict::new_bound(py);
    let _ = d.set_item("valence", e.valence);
    let _ = d.set_item("arousal", e.arousal);
    let _ = d.set_item("anger", e.anger);
    let _ = d.set_item("surprise", e.surprise);
    let _ = d.set_item("tension", e.tension);
    d
}

// ----- Sinks ----------------------------------------------------------------

/// Trait object held by the harness builder; concrete impls below.
type DynSink = Arc<dyn AvatarSink>;

#[pyclass(name = "AvatarSink", module = "atomr_agents._native.avatar")]
#[derive(Clone)]
pub struct PyAvatarSink {
    pub(crate) sink: DynSink,
    pub(crate) label: String,
}

#[pymethods]
impl PyAvatarSink {
    fn kind(&self) -> &'static str {
        match self.sink.kind() {
            SinkKind::LiveLinkUdp => "livelink_udp",
            SinkKind::Audio2Face => "audio2face",
            SinkKind::MockCapture => "mock_capture",
            SinkKind::LiveLinkPlugin => "livelink_plugin",
        }
    }

    fn __repr__(&self) -> String {
        format!("AvatarSink(kind={}, label={})", self.kind(), self.label)
    }
}

/// In-memory sink that captures every emitted frame. Always
/// available, even on aarch64.
#[pyclass(name = "CapturingSink", module = "atomr_agents._native.avatar")]
pub struct PyCapturingSink {
    inner:
        Arc<atomr_agents_avatar_harness::test_support::CapturingSink>,
}

#[pymethods]
impl PyCapturingSink {
    #[new]
    fn new() -> Self {
        Self {
            inner: Arc::new(
                atomr_agents_avatar_harness::test_support::CapturingSink::new(),
            ),
        }
    }

    /// Get the sink handle to pass to `AvatarHarness.attach_sink(...)`.
    fn as_sink(&self) -> PyAvatarSink {
        PyAvatarSink {
            sink: self.inner.clone() as DynSink,
            label: "mock_capture".into(),
        }
    }

    /// Drain captured frames (in order). The internal buffer is
    /// cleared on each call.
    fn drain<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let frames = self.inner.frames.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut g = frames.lock().await;
            let out: Vec<PyAvatarFrame> = g
                .drain(..)
                .map(|f| PyAvatarFrame { inner: f })
                .collect();
            Python::with_gil(|py| Ok(out.into_py(py)))
        })
    }
}

// ----- LiveLink sink (x86_64 + avatar-livelink feature only) ---------------

#[cfg(all(
    target_arch = "x86_64",
    feature = "avatar-livelink"
))]
#[pyclass(name = "LiveLinkSink", module = "atomr_agents._native.avatar")]
pub struct PyLiveLinkSink {
    inner: Arc<atomr_agents_avatar_provider_livelink::LiveLinkSink>,
    label: String,
}

#[cfg(all(
    target_arch = "x86_64",
    feature = "avatar-livelink"
))]
#[pymethods]
impl PyLiveLinkSink {
    #[new]
    #[pyo3(signature = (addr, *, max_fps = 60, label = None))]
    fn new(addr: &str, max_fps: u32, label: Option<String>) -> PyResult<Self> {
        let parsed: std::net::SocketAddr = addr.parse().map_err(val_err)?;
        let cfg = atomr_agents_avatar_provider_livelink::LiveLinkConfig {
            addr: parsed,
            bind: None,
            max_fps,
            label: label.clone().unwrap_or_else(|| "livelink-udp".into()),
        };
        Ok(Self {
            inner: Arc::new(atomr_agents_avatar_provider_livelink::LiveLinkSink::new(cfg)),
            label: label.unwrap_or_else(|| "livelink-udp".into()),
        })
    }

    fn as_sink(&self) -> PyAvatarSink {
        PyAvatarSink {
            sink: self.inner.clone() as DynSink,
            label: self.label.clone(),
        }
    }
}

// ----- Python-side inference adapter ---------------------------------------

/// Bridge that lets the harness call into a Python `async` callable
/// for cognition. Pattern mirrors `crates/agent/src/inference.rs`'s
/// `InferenceClient` — but routed through whatever Python wrapper
/// the caller has built on top of `atomr-infer`.
pub struct PyInferenceClient {
    callable: Py<PyAny>,
}

#[async_trait]
impl AvatarInferenceClient for PyInferenceClient {
    async fn complete(&self, batch: ExecuteBatch) -> Result<String> {
        // Marshal the ExecuteBatch into a JSON-shaped dict for Python.
        let batch_json =
            serde_json::to_value(&batch).map_err(|e| AvatarError::cognition(e.to_string()))?;
        let coro = Python::with_gil(|py| -> PyResult<Py<PyAny>> {
            let dict = crate::conv::json_to_py(py, &batch_json)?;
            let result = self.callable.bind(py).call1((dict,))?;
            Ok(result.into_py(py))
        })
        .map_err(|e| AvatarError::cognition(e.to_string()))?;

        let fut = Python::with_gil(|py| -> PyResult<_> {
            let bound = coro.bind(py);
            pyo3_async_runtimes::tokio::into_future(bound.clone())
        })
        .map_err(|e| AvatarError::cognition(e.to_string()))?;

        let py_out = fut
            .await
            .map_err(|e| AvatarError::cognition(e.to_string()))?;
        let text = Python::with_gil(|py| py_out.bind(py).extract::<String>())
            .map_err(|e| AvatarError::cognition(e.to_string()))?;
        Ok(text)
    }
}

// ----- AvatarHarness binding ----------------------------------------------

#[pyclass(name = "AvatarHarness", module = "atomr_agents._native.avatar")]
pub struct PyAvatarHarness {
    pub(crate) inner: Arc<AvatarHarness>,
}

#[pymethods]
impl PyAvatarHarness {
    /// Build a new harness.
    ///
    /// Args:
    ///   inference: an `async def fn(batch_dict) -> str` callable
    ///     that runs the inference batch (typically wrapping
    ///     `atomr-infer`) and returns the assistant text.
    ///   tts: a `TextToSpeech` instance from the existing tts bindings.
    ///   voice_id: the backend voice library identifier the TTS should
    ///     use (e.g. `"alloy"` for OpenAI, `"rachel"` for ElevenLabs).
    ///   frame_rate: output frame cadence in Hz. Default `60`.
    ///   model: model id to pass to the inference client. Default the
    ///     cognition config default.
    ///   persona: persona prompt for the cognition layer.
    #[new]
    #[pyo3(signature = (inference, tts, voice_id, *, frame_rate = 60, model = None, persona = None))]
    fn new(
        inference: Py<PyAny>,
        tts: Py<PyTextToSpeech>,
        voice_id: String,
        frame_rate: u8,
        model: Option<String>,
        persona: Option<String>,
    ) -> PyResult<Self> {
        let tts_dyn: DynTextToSpeech =
            Python::with_gil(|py| tts.borrow(py).inner.clone());
        let py_client: Arc<dyn AvatarInferenceClient> =
            Arc::new(PyInferenceClient { callable: inference });

        let mut cog_cfg = CognitionConfig::default();
        if let Some(m) = model {
            cog_cfg.model = m;
        }
        if let Some(p) = persona {
            cog_cfg.persona_prompt = p;
        }

        let harness = AvatarHarnessBuilder::new()
            .with_inference(py_client)
            .with_tts(tts_dyn, VoiceRef::library(voice_id))
            .with_cognition_config(cog_cfg)
            .with_config(AvatarHarnessConfig {
                sync: SyncConfig {
                    frame_rate,
                    apply_emotion: true,
                },
                ..Default::default()
            })
            .build()
            .map_err(val_err)?;

        Ok(Self {
            inner: Arc::new(harness),
        })
    }

    fn attach_sink<'py>(
        &self,
        py: Python<'py>,
        sink: Py<PyAvatarSink>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let sink = Python::with_gil(|py| sink.borrow(py).sink.clone());
        let _ = py;
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner.attach_sink(sink).await.map_err(rt_err)
        })
    }

    fn user_said<'py>(&self, py: Python<'py>, text: String) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner.user_said(text).await.map_err(rt_err)
        })
    }

    fn speak_text<'py>(&self, py: Python<'py>, text: String) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner.speak_text(text).await.map_err(rt_err)
        })
    }

    /// Snapshot the most-recent agent intent. Returns `None` until
    /// at least one turn has run.
    fn last_intent<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let intent = inner.last_intent().await;
            Python::with_gil(|py| {
                Ok(match intent {
                    Some(i) => intent_to_dict(py, &i).into_py(py),
                    None => py.None(),
                })
            })
        })
    }

    /// Snapshot the running emotion vector (synchronous).
    fn emotion<'py>(&self, py: Python<'py>) -> Bound<'py, PyDict> {
        emotion_to_dict(py, &self.inner.emotion())
    }

    fn reset_emotion(&self) {
        self.inner.reset_emotion();
    }

    fn shutdown<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner.shutdown().await.map_err(rt_err)
        })
    }
}

fn intent_to_dict<'py>(py: Python<'py>, i: &AgentIntentPacket) -> Bound<'py, PyDict> {
    let d = PyDict::new_bound(py);
    let _ = d.set_item("response_text", i.response_text.clone());
    let emo = PyDict::new_bound(py);
    let _ = emo.set_item("valence", i.emotion_delta.valence);
    let _ = emo.set_item("arousal", i.emotion_delta.arousal);
    let _ = emo.set_item("anger", i.emotion_delta.anger);
    let _ = emo.set_item("surprise", i.emotion_delta.surprise);
    let _ = emo.set_item("tension", i.emotion_delta.tension);
    let _ = d.set_item("emotion_delta", emo);
    let _ = d.set_item(
        "gesture",
        i.gesture.map(|g| match g {
            atomr_agents_avatar_harness::GestureHint::Nod => "nod",
            atomr_agents_avatar_harness::GestureHint::Shake => "shake",
            atomr_agents_avatar_harness::GestureHint::Shrug => "shrug",
            atomr_agents_avatar_harness::GestureHint::Wave => "wave",
            atomr_agents_avatar_harness::GestureHint::Point => "point",
            atomr_agents_avatar_harness::GestureHint::Idle => "idle",
        }),
    );
    d
}

// ----- registration --------------------------------------------------------

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "avatar")?;
    m.add_class::<PyAvatarHarness>()?;
    m.add_class::<PyAvatarFrame>()?;
    m.add_class::<PyAvatarSink>()?;
    m.add_class::<PyCapturingSink>()?;
    #[cfg(all(target_arch = "x86_64", feature = "avatar-livelink"))]
    m.add_class::<PyLiveLinkSink>()?;
    parent.add_submodule(&m)?;
    let _ = (py, BlendshapeWeights::zero(), SmpteTimecode::from_frame_index(0, 60));
    Ok(())
}
