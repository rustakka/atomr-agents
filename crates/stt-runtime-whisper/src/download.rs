//! Optional helper for fetching ggml whisper weights into the OS
//! cache dir. Gated on the `download-models` feature so the
//! default build doesn't pull `reqwest`.

use std::path::PathBuf;

use atomr_agents_stt_core::{Result, SttError};
use tokio::io::AsyncWriteExt;

use crate::config::WhisperModel;

const HF_BASE: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/";

/// Default cache location: `<dirs::cache_dir>/atomr-agents/whisper/`.
pub fn default_cache_dir() -> Result<PathBuf> {
    let base = dirs::cache_dir()
        .ok_or_else(|| SttError::internal("dirs::cache_dir unavailable"))?;
    let dir = base.join("atomr-agents").join("whisper");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Download `model` from Hugging Face into `cache_dir` (or the
/// default cache dir if `None`). Returns the local path; if the
/// file already exists, no network call is made.
pub async fn download_model(
    model: WhisperModel,
    cache_dir: Option<PathBuf>,
) -> Result<PathBuf> {
    let dir = match cache_dir {
        Some(d) => d,
        None => default_cache_dir()?,
    };
    let target = dir.join(model.ggml_filename());
    if target.exists() {
        return Ok(target);
    }
    let url = format!("{}{}", HF_BASE, model.ggml_filename());
    tracing::info!(%url, ?target, "downloading whisper model");

    let resp = reqwest::get(&url)
        .await
        .map_err(|e| SttError::transport(format!("download GET: {e}")))?;
    if !resp.status().is_success() {
        return Err(SttError::Backend {
            status: resp.status().as_u16(),
            message: format!("model download failed: {url}"),
        });
    }
    let mut stream = resp.bytes_stream();
    let mut file = tokio::fs::File::create(&target).await?;
    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|e| SttError::transport(format!("download read: {e}")))?;
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    Ok(target)
}
