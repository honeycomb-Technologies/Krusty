use anyhow::{bail, Context, Result};
use futures::StreamExt;
use std::path::Path;
use url::Url;

use super::PluginManager;

pub(super) async fn read_remote_bytes_with_limit(
    manager: &PluginManager,
    url: &Url,
    max_bytes: usize,
    purpose: &str,
) -> Result<Vec<u8>> {
    let response = manager
        .http_client
        .get(url.clone())
        .send()
        .await
        .with_context(|| format!("failed to fetch {} from {}", purpose, url))?
        .error_for_status()
        .with_context(|| format!("{} request failed for {}", purpose, url))?;

    if let Some(content_length) = response.content_length() {
        if content_length > max_bytes as u64 {
            bail!(
                "{} at {} exceeds maximum allowed size ({} bytes > {} bytes)",
                purpose,
                url,
                content_length,
                max_bytes
            );
        }
    }

    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("failed to read HTTP response chunk")?;
        if bytes.len() + chunk.len() > max_bytes {
            bail!(
                "{} at {} exceeds maximum allowed size (>{} bytes)",
                purpose,
                url,
                max_bytes
            );
        }
        bytes.extend_from_slice(&chunk);
    }

    Ok(bytes)
}

pub(super) async fn read_local_bytes_with_limit(
    path: &Path,
    max_bytes: usize,
    purpose: &str,
) -> Result<Vec<u8>> {
    let metadata = tokio::fs::metadata(path)
        .await
        .with_context(|| format!("failed to stat {} {}", purpose, path.display()))?;
    if metadata.len() > max_bytes as u64 {
        bail!(
            "{} {} exceeds maximum allowed size ({} bytes > {} bytes)",
            purpose,
            path.display(),
            metadata.len(),
            max_bytes
        );
    }

    tokio::fs::read(path)
        .await
        .with_context(|| format!("failed to read {} {}", purpose, path.display()))
}
