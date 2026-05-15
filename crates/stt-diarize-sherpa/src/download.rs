//! Optional helper for downloading sherpa-onnx diarization models
//! into the OS cache dir. Gated on the `download-models` feature.

use std::path::PathBuf;

use atomr_agents_stt_core::{Result, SttError};
use tokio::io::AsyncWriteExt;

/// Default cache location: `<dirs::cache_dir>/atomr-agents/sherpa/`.
pub fn default_cache_dir() -> Result<PathBuf> {
    let base = dirs::cache_dir().ok_or_else(|| SttError::internal("dirs::cache_dir unavailable"))?;
    let dir = base.join("atomr-agents").join("sherpa");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Generic downloader. Returns the destination path; if the file
/// already exists, no network call is made. Used by the project's
/// docs to fetch the recommended segmentation + embedding models.
pub async fn download_to(url: &str, target: PathBuf) -> Result<PathBuf> {
    if target.exists() {
        return Ok(target);
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    tracing::info!(%url, ?target, "downloading sherpa model");
    let resp = reqwest::get(url)
        .await
        .map_err(|e| SttError::transport(format!("download GET: {e}")))?;
    if !resp.status().is_success() {
        return Err(SttError::Backend {
            status: resp.status().as_u16(),
            message: format!("download {url} failed"),
        });
    }
    let mut stream = resp.bytes_stream();
    let mut file = tokio::fs::File::create(&target).await?;
    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| SttError::transport(format!("download read: {e}")))?;
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    Ok(target)
}
