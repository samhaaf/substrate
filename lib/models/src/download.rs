//! Resumable model download pipeline.
//!
//! Supports three source schemes:
//! - `hf:owner/repo/filename.gguf` — Hugging Face (resolve to HTTPS, then stream)
//! - `https://...`                 — Direct HTTPS download with `Range` resume support
//! - `file:///absolute/path`       — Local file (copy, no download)
//!
//! Downloads stream to a `<dest>.partial` file and are atomically renamed on
//! completion. If a `.partial` file already exists, the download resumes from
//! the existing byte offset using an HTTP `Range` header.

use std::path::{Path, PathBuf};

use futures::StreamExt;
use tokio::io::AsyncWriteExt;

use substrate_types::{Result, SubstrateError};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Download a model to `dest_path`, resuming if a `.partial` file exists.
///
/// Returns the number of bytes in the final file (total, not just this chunk).
///
/// On completion, renames `<dest_path>.partial` → `dest_path`.
/// Progress is logged via `tracing::info!`.
pub async fn download_model(
    source: &str,
    dest_path: &Path,
    hf_token: Option<&str>,
) -> Result<u64> {
    match source.split_once(':') {
        Some(("hf", hf_path)) => download_hf(hf_path, dest_path, hf_token).await,
        Some(("https", _)) | Some(("http", _)) => download_https(source, dest_path, None).await,
        Some(("file", local_path)) => {
            copy_local(local_path.trim_start_matches("//"), dest_path).await
        }
        _ => Err(SubstrateError::Config(format!(
            "unsupported model source scheme: {source}"
        ))),
    }
}

// ---------------------------------------------------------------------------
// HuggingFace source
// ---------------------------------------------------------------------------

/// Resolve a `hf:owner/repo/filename` reference to an HTTPS URL and download.
///
/// Expected format: `hf:<org>/<repo>/<filename>`
/// Resolves to:     `https://huggingface.co/<org>/<repo>/resolve/main/<filename>`
async fn download_hf(hf_path: &str, dest_path: &Path, token: Option<&str>) -> Result<u64> {
    // Parse: <org>/<repo>/<filename>
    // splitn(3) so a filename with path separators still works as the last segment.
    let parts: Vec<&str> = hf_path.splitn(3, '/').collect();
    if parts.len() < 3 {
        return Err(SubstrateError::DownloadFailed {
            model: format!("hf:{hf_path}"),
            message: "hf: URI must be hf:<org>/<repo>/<filename>".into(),
        });
    }

    let url = format!(
        "https://huggingface.co/{}/{}/resolve/main/{}",
        parts[0], parts[1], parts[2]
    );

    tracing::info!("resolved hf:{hf_path} -> {url}");
    download_https(&url, dest_path, token).await
}

// ---------------------------------------------------------------------------
// HTTPS source (with resume support)
// ---------------------------------------------------------------------------

/// HTTPS download with `Range` header for resume support.
///
/// Streams into `<dest_path>.partial`, then renames to `dest_path`.
async fn download_https(url: &str, dest_path: &Path, token: Option<&str>) -> Result<u64> {
    let partial_path = partial_path(dest_path);

    // Check for existing partial download.
    let existing_bytes = match tokio::fs::metadata(&partial_path).await {
        Ok(meta) => meta.len(),
        Err(_) => 0,
    };

    let client = reqwest::Client::new();
    let mut request = client.get(url);

    if let Some(tok) = token {
        request = request.header("Authorization", format!("Bearer {tok}"));
    }

    if existing_bytes > 0 {
        request = request.header("Range", format!("bytes={existing_bytes}-"));
        tracing::info!(
            "resuming download of {} from byte {}",
            dest_path.display(),
            existing_bytes
        );
    } else {
        tracing::info!("downloading {} -> {}", url, dest_path.display());
    }

    let resp = request.send().await.map_err(|e| SubstrateError::DownloadFailed {
        model: url.to_string(),
        message: e.to_string(),
    })?;

    let status = resp.status();
    if !status.is_success() && status != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(SubstrateError::DownloadFailed {
            model: url.to_string(),
            message: format!("HTTP {status}"),
        });
    }

    // If we got 200 (not 206), the server ignored our Range header — restart.
    let append = status == reqwest::StatusCode::PARTIAL_CONTENT && existing_bytes > 0;

    // Ensure parent directory exists.
    if let Some(parent) = partial_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let mut file = if append {
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(&partial_path)
            .await?
    } else {
        tokio::fs::File::create(&partial_path).await?
    };

    let mut bytes_written: u64 = if append { existing_bytes } else { 0 };
    let mut stream = resp.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let data = chunk.map_err(|e| SubstrateError::DownloadFailed {
            model: url.to_string(),
            message: e.to_string(),
        })?;
        file.write_all(&data).await?;
        bytes_written += data.len() as u64;
    }

    file.flush().await?;
    drop(file);

    // Atomic rename partial → final.
    tokio::fs::rename(&partial_path, dest_path).await?;

    tracing::info!(
        "download complete: {} ({} bytes)",
        dest_path.display(),
        bytes_written
    );

    Ok(bytes_written)
}

// ---------------------------------------------------------------------------
// Local file source
// ---------------------------------------------------------------------------

/// Copy a local file to the destination path.
///
/// The `file:///` prefix has already been stripped by the caller.
async fn copy_local(src: &str, dest_path: &Path) -> Result<u64> {
    let src_path = Path::new(src);
    if !src_path.exists() {
        return Err(SubstrateError::Config(format!(
            "local model source does not exist: {src}"
        )));
    }

    if let Some(parent) = dest_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let bytes = tokio::fs::copy(src_path, dest_path).await?;
    tracing::info!(
        "copied local model {} -> {} ({} bytes)",
        src,
        dest_path.display(),
        bytes
    );
    Ok(bytes)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return the `.partial` file path for a given destination.
fn partial_path(dest: &Path) -> PathBuf {
    let mut p = dest.to_path_buf();
    let filename = p
        .file_name()
        .map(|n| {
            let mut s = n.to_os_string();
            s.push(".partial");
            s
        })
        .unwrap_or_else(|| "download.partial".into());
    p.set_file_name(filename);
    p
}
