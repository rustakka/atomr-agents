//! PyO3 bindings for voice-adjacent traits that don't fit neatly into
//! the STT or TTS surfaces on their own:
//!
//! - [`Diarizer`](atomr_agents_stt_diarize_sherpa::Diarizer) — turn a
//!   PCM buffer into a list of `DiarizationSpan`s. Two backends are
//!   exposed: `mock_diarizer()` (deterministic round-robin) and
//!   `sherpa_diarizer(...)` (feature-gated; raises
//!   `NotImplementedError` when built without the
//!   `stt-diarize-sherpa-onnx` feature).
//! - [`Vad`](atomr_agents_stt_voice::Vad) — frame-level voice-activity
//!   detector. `energy_vad(threshold)` (always available) and
//!   `silero_vad(model_path)` (gated by `stt-vad-silero`).
//! - [`Phonemizer`](atomr_agents_tts_core::Phonemizer) — text → IPA +
//!   per-token list. `mock_phonemizer()` ships always; backends like
//!   espeak-ng live in sibling crates.
//!
//! Each trait also gets a `*_from_factory(key)` entry point that
//! materialises a dyn handle from a Python implementation registered
//! via `guest.register_diarizer_factory` / `register_vad_factory` /
//! `register_phonemizer_factory`.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_stt_core::{PcmBuffer, Result as SttResult, SttError};
use atomr_agents_stt_diarize_sherpa::{
    DiarizationSpan, Diarizer, MockDiarizer, SherpaDiarizer, SherpaDiarizerConfig,
};
use atomr_agents_stt_voice::{EnergyVad, Vad};
use atomr_agents_tts_core::{MockPhonemizer, PhonemizedText, Phonemizer};
use parking_lot::Mutex;
use pyo3::exceptions::PyNotImplementedError;
use pyo3::prelude::*;
use pyo3::types::PyList;

use crate::strategy::await_if_coro;

// ============================================================================
// PyDiarizer
// ============================================================================

#[pyclass(name = "DiarizationSpan", module = "atomr_agents._native.voice_extras")]
#[derive(Clone)]
pub struct PyDiarizationSpan {
    pub(crate) inner: DiarizationSpan,
}

#[pymethods]
impl PyDiarizationSpan {
    #[new]
    #[pyo3(signature = (start_ms, end_ms, speaker_id, confidence=None))]
    fn new(start_ms: u32, end_ms: u32, speaker_id: u8, confidence: Option<f32>) -> Self {
        Self {
            inner: DiarizationSpan {
                start_ms,
                end_ms,
                speaker_id,
                confidence,
            },
        }
    }

    #[getter]
    fn start_ms(&self) -> u32 {
        self.inner.start_ms
    }
    #[getter]
    fn end_ms(&self) -> u32 {
        self.inner.end_ms
    }
    #[getter]
    fn speaker_id(&self) -> u8 {
        self.inner.speaker_id
    }
    #[getter]
    fn confidence(&self) -> Option<f32> {
        self.inner.confidence
    }

    fn __repr__(&self) -> String {
        format!(
            "DiarizationSpan(start_ms={}, end_ms={}, speaker={}, confidence={:?})",
            self.inner.start_ms,
            self.inner.end_ms,
            self.inner.speaker_id,
            self.inner.confidence,
        )
    }
}

#[pyclass(name = "Diarizer", module = "atomr_agents._native.voice_extras")]
#[derive(Clone)]
pub struct PyDiarizer {
    pub(crate) inner: Arc<dyn Diarizer>,
}

#[pymethods]
impl PyDiarizer {
    /// `diarize(samples, sample_rate, channels=1)` — returns a list of
    /// `DiarizationSpan`. Awaitable.
    #[pyo3(signature = (samples, sample_rate, channels=1))]
    fn diarize<'py>(
        &self,
        py: Python<'py>,
        samples: Vec<f32>,
        sample_rate: u32,
        channels: u16,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let pcm = PcmBuffer::new(samples, sample_rate, channels);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let spans = inner.diarize(&pcm).await.map_err(crate::errors::map)?;
            Ok(spans
                .into_iter()
                .map(|s| PyDiarizationSpan { inner: s })
                .collect::<Vec<_>>())
        })
    }

    fn __repr__(&self) -> String {
        "Diarizer(handle)".into()
    }
}

// ----- Python guest adapter -------------------------------------------------

pub(crate) struct PyDiarizerAdapter {
    target: Arc<PyObject>,
}

#[async_trait]
impl Diarizer for PyDiarizerAdapter {
    async fn diarize(&self, pcm: &PcmBuffer) -> SttResult<Vec<DiarizationSpan>> {
        let target = self.target.clone();
        let samples = pcm.samples.clone();
        let sample_rate = pcm.sample_rate;
        let channels = pcm.channels;
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let instance: Bound<'_, PyAny> = if bound.hasattr("diarize")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance
                .getattr("diarize")?
                .call1((samples, sample_rate, channels))?;
            Ok(r.unbind())
        })
        .map_err(|e| SttError::internal(format!("py diarizer: {e}")))?;
        let final_val = await_if_coro(coro_or_val)
            .await
            .map_err(|e| SttError::internal(format!("py diarizer await: {e}")))?;
        let spans = Python::with_gil(|py| -> PyResult<Vec<DiarizationSpan>> {
            let bound = final_val.bind(py);
            let mut out: Vec<DiarizationSpan> = Vec::new();
            for item in bound.iter()? {
                let item = item?;
                // Accept either PyDiarizationSpan or a 4-tuple
                // (start_ms, end_ms, speaker_id, confidence).
                if let Ok(span) = item.extract::<PyDiarizationSpan>() {
                    out.push(span.inner);
                    continue;
                }
                if let Ok((start_ms, end_ms, speaker_id, confidence)) =
                    item.extract::<(u32, u32, u8, Option<f32>)>()
                {
                    out.push(DiarizationSpan {
                        start_ms,
                        end_ms,
                        speaker_id,
                        confidence,
                    });
                    continue;
                }
                if let Ok((start_ms, end_ms, speaker_id)) = item.extract::<(u32, u32, u8)>() {
                    out.push(DiarizationSpan {
                        start_ms,
                        end_ms,
                        speaker_id,
                        confidence: None,
                    });
                    continue;
                }
                return Err(pyo3::exceptions::PyTypeError::new_err(
                    "diarize() must return an iterable of DiarizationSpan or \
                     (start_ms, end_ms, speaker_id[, confidence]) tuples",
                ));
            }
            Ok(out)
        })
        .map_err(|e| SttError::internal(format!("py diarizer result: {e}")))?;
        Ok(spans)
    }
}

// ----- Diarizer factories ---------------------------------------------------

#[pyfunction]
#[pyo3(signature = (chunk_secs=2.5, n_speakers=2))]
fn mock_diarizer(chunk_secs: f32, n_speakers: u8) -> PyDiarizer {
    PyDiarizer {
        inner: Arc::new(MockDiarizer::new(chunk_secs, n_speakers)),
    }
}

#[pyfunction]
#[pyo3(signature = (segmentation_model, embedding_model, num_speakers=None, use_gpu=false))]
fn sherpa_diarizer(
    segmentation_model: PathBuf,
    embedding_model: PathBuf,
    num_speakers: Option<u8>,
    use_gpu: bool,
) -> PyResult<PyDiarizer> {
    #[cfg(feature = "stt-diarize-sherpa-onnx")]
    {
        let config = SherpaDiarizerConfig {
            segmentation_model,
            embedding_model,
            num_speakers,
            use_gpu,
        };
        let d = SherpaDiarizer::new(config).map_err(crate::errors::map)?;
        Ok(PyDiarizer {
            inner: Arc::new(d),
        })
    }
    #[cfg(not(feature = "stt-diarize-sherpa-onnx"))]
    {
        // Keep references "used" to avoid dead_code / unused_variables
        // warnings when the feature is off.
        let _ = (segmentation_model, embedding_model, num_speakers, use_gpu);
        let _ = std::marker::PhantomData::<(SherpaDiarizer, SherpaDiarizerConfig)>;
        Err(PyNotImplementedError::new_err(
            "sherpa_diarizer requires building atomr-agents-py-bindings with \
             --features stt-diarize-sherpa-onnx",
        ))
    }
}

#[pyfunction]
fn diarizer_from_factory(key: String) -> PyResult<PyDiarizer> {
    let target = crate::guest::must_lookup("diarizer", &key)?;
    Ok(PyDiarizer {
        inner: Arc::new(PyDiarizerAdapter { target }),
    })
}

// ============================================================================
// PyVad
// ============================================================================

#[pyclass(name = "Vad", module = "atomr_agents._native.voice_extras")]
#[derive(Clone)]
pub struct PyVad {
    /// `Vad::is_speech` takes `&mut self`. We wrap behind a mutex so
    /// the Python-facing handle stays `Clone + Sync`.
    pub(crate) inner: Arc<Mutex<Box<dyn Vad>>>,
}

#[pymethods]
impl PyVad {
    /// `is_speech(frame, sample_rate)` — returns `True` if the frame
    /// is likely speech. `frame` is a list of f32 PCM samples (mono).
    fn is_speech(&self, frame: Vec<f32>, sample_rate: u32) -> bool {
        let mut g = self.inner.lock();
        g.is_speech(&frame, sample_rate)
    }

    fn __repr__(&self) -> String {
        "Vad(handle)".into()
    }
}

// ----- Python guest adapter -------------------------------------------------

pub(crate) struct PyVadAdapter {
    target: Arc<PyObject>,
}

impl Vad for PyVadAdapter {
    fn is_speech(&mut self, frame: &[f32], sample_rate: u32) -> bool {
        let target = self.target.clone();
        let frame_vec = frame.to_vec();
        Python::with_gil(|py| -> PyResult<bool> {
            let bound = target.bind(py);
            let instance: Bound<'_, PyAny> = if bound.hasattr("is_speech")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance
                .getattr("is_speech")?
                .call1((frame_vec, sample_rate))?;
            r.extract::<bool>()
        })
        .unwrap_or(false)
    }
}

// ----- Vad factories --------------------------------------------------------

#[pyfunction]
#[pyo3(signature = (threshold=0.01))]
fn energy_vad(threshold: f32) -> PyVad {
    let v: Box<dyn Vad> = Box::new(EnergyVad { threshold });
    PyVad {
        inner: Arc::new(Mutex::new(v)),
    }
}

#[pyfunction]
#[pyo3(signature = (_model_path, sample_rate=16_000, chunk_size=512, threshold=0.5))]
fn silero_vad(
    _model_path: PathBuf,
    sample_rate: u32,
    chunk_size: usize,
    threshold: f32,
) -> PyResult<PyVad> {
    #[cfg(feature = "stt-vad-silero")]
    {
        // The `voice_activity_detector` crate ships its own bundled
        // model; the supplied path is accepted for API symmetry with
        // other backends but the SileroVad constructor doesn't take
        // it. Reference the variables to silence unused warnings.
        let _ = _model_path;
        let v: Box<dyn Vad> = Box::new(atomr_agents_stt_voice::SileroVad::new(
            sample_rate,
            chunk_size,
            threshold,
        ));
        Ok(PyVad {
            inner: Arc::new(Mutex::new(v)),
        })
    }
    #[cfg(not(feature = "stt-vad-silero"))]
    {
        let _ = (sample_rate, chunk_size, threshold);
        Err(PyNotImplementedError::new_err(
            "silero_vad requires building atomr-agents-py-bindings with \
             --features stt-vad-silero",
        ))
    }
}

#[pyfunction]
fn vad_from_factory(key: String) -> PyResult<PyVad> {
    let target = crate::guest::must_lookup("vad", &key)?;
    let v: Box<dyn Vad> = Box::new(PyVadAdapter { target });
    Ok(PyVad {
        inner: Arc::new(Mutex::new(v)),
    })
}

// ============================================================================
// PyPhonemizer
// ============================================================================

#[pyclass(name = "PhonemizedText", module = "atomr_agents._native.voice_extras")]
#[derive(Clone)]
pub struct PyPhonemizedText {
    pub(crate) inner: PhonemizedText,
}

#[pymethods]
impl PyPhonemizedText {
    #[new]
    fn new(ipa: String, tokens: Vec<String>) -> Self {
        Self {
            inner: PhonemizedText { ipa, tokens },
        }
    }

    #[getter]
    fn ipa(&self) -> &str {
        &self.inner.ipa
    }

    #[getter]
    fn tokens(&self) -> Vec<String> {
        self.inner.tokens.clone()
    }

    fn __repr__(&self) -> String {
        format!(
            "PhonemizedText(ipa={:?}, tokens={})",
            self.inner.ipa,
            self.inner.tokens.len()
        )
    }
}

#[pyclass(name = "Phonemizer", module = "atomr_agents._native.voice_extras")]
#[derive(Clone)]
pub struct PyPhonemizer {
    pub(crate) inner: Arc<dyn Phonemizer>,
}

#[pymethods]
impl PyPhonemizer {
    /// `phonemize(text, language="en-us")` — returns a `PhonemizedText`.
    /// Awaitable.
    #[pyo3(signature = (text, language="en-us"))]
    fn phonemize<'py>(
        &self,
        py: Python<'py>,
        text: String,
        language: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let language = language.to_string();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let pt = inner
                .phonemize(&text, &language)
                .await
                .map_err(crate::errors::map)?;
            Ok(PyPhonemizedText { inner: pt })
        })
    }

    fn __repr__(&self) -> String {
        "Phonemizer(handle)".into()
    }
}

// ----- Python guest adapter -------------------------------------------------

pub(crate) struct PyPhonemizerAdapter {
    target: Arc<PyObject>,
}

#[async_trait]
impl Phonemizer for PyPhonemizerAdapter {
    async fn phonemize(&self, text: &str, language: &str) -> SttResult<PhonemizedText> {
        let target = self.target.clone();
        let text = text.to_string();
        let language = language.to_string();
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let instance: Bound<'_, PyAny> = if bound.hasattr("phonemize")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("phonemize")?.call1((text, language))?;
            Ok(r.unbind())
        })
        .map_err(|e| SttError::internal(format!("py phonemizer: {e}")))?;
        let final_val = await_if_coro(coro_or_val)
            .await
            .map_err(|e| SttError::internal(format!("py phonemizer await: {e}")))?;
        let pt = Python::with_gil(|py| -> PyResult<PhonemizedText> {
            let bound = final_val.bind(py);
            // Accept either PyPhonemizedText or a {"ipa": str, "tokens": list[str]}
            // dict, or a (ipa, tokens) tuple.
            if let Ok(pt) = bound.extract::<PyPhonemizedText>() {
                return Ok(pt.inner);
            }
            if let Ok((ipa, tokens)) = bound.extract::<(String, Vec<String>)>() {
                return Ok(PhonemizedText { ipa, tokens });
            }
            if let Ok(d) = bound.downcast::<pyo3::types::PyDict>() {
                let ipa: String = d
                    .get_item("ipa")?
                    .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err("'ipa'"))?
                    .extract()?;
                let tokens_obj = d
                    .get_item("tokens")?
                    .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err("'tokens'"))?;
                let tokens: Vec<String> = tokens_obj
                    .downcast::<PyList>()?
                    .iter()
                    .map(|t| t.extract::<String>())
                    .collect::<PyResult<Vec<_>>>()?;
                return Ok(PhonemizedText { ipa, tokens });
            }
            Err(pyo3::exceptions::PyTypeError::new_err(
                "phonemize() must return PhonemizedText, (ipa, tokens), or a dict",
            ))
        })
        .map_err(|e| SttError::internal(format!("py phonemizer result: {e}")))?;
        Ok(pt)
    }
}

// ----- Phonemizer factories -------------------------------------------------

#[pyfunction]
fn mock_phonemizer() -> PyPhonemizer {
    PyPhonemizer {
        inner: Arc::new(MockPhonemizer),
    }
}

#[pyfunction]
fn phonemizer_from_factory(key: String) -> PyResult<PyPhonemizer> {
    let target = crate::guest::must_lookup("phonemizer", &key)?;
    Ok(PyPhonemizer {
        inner: Arc::new(PyPhonemizerAdapter { target }),
    })
}

// ============================================================================
// Module registration
// ============================================================================

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "voice_extras")?;
    m.add_class::<PyDiarizationSpan>()?;
    m.add_class::<PyDiarizer>()?;
    m.add_class::<PyVad>()?;
    m.add_class::<PyPhonemizedText>()?;
    m.add_class::<PyPhonemizer>()?;
    m.add_function(wrap_pyfunction!(mock_diarizer, &m)?)?;
    m.add_function(wrap_pyfunction!(sherpa_diarizer, &m)?)?;
    m.add_function(wrap_pyfunction!(diarizer_from_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(energy_vad, &m)?)?;
    m.add_function(wrap_pyfunction!(silero_vad, &m)?)?;
    m.add_function(wrap_pyfunction!(vad_from_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(mock_phonemizer, &m)?)?;
    m.add_function(wrap_pyfunction!(phonemizer_from_factory, &m)?)?;
    parent.add_submodule(&m)?;
    Ok(())
}
