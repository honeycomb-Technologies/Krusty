use anyhow::{bail, Context, Result};
use futures::StreamExt;
use std::{path::Path, time::Duration};
use tokio::{io::AsyncReadExt, time::timeout};
use url::Url;

use super::PluginManager;

const REMOTE_CONNECT_AND_HEADERS_TIMEOUT: Duration = Duration::from_secs(20);
const REMOTE_REQUEST_TIMEOUT: Duration = Duration::from_secs(90);
const REMOTE_BODY_IDLE_TIMEOUT: Duration = Duration::from_secs(15);

pub(super) async fn read_remote_bytes_with_limit(
    manager: &PluginManager,
    url: &Url,
    max_bytes: usize,
    purpose: &str,
) -> Result<Vec<u8>> {
    if url.scheme() != "https" {
        bail!(
            "{} URL must use HTTPS (local development may use a file path or file:// URL): {}",
            purpose,
            url
        );
    }

    let request = manager
        .http_client
        .get(url.clone())
        .timeout(REMOTE_REQUEST_TIMEOUT);
    let response = timeout(REMOTE_CONNECT_AND_HEADERS_TIMEOUT, request.send())
        .await
        .with_context(|| {
            format!(
                "timed out connecting to {} or waiting for {} response headers after {} seconds",
                url,
                purpose,
                REMOTE_CONNECT_AND_HEADERS_TIMEOUT.as_secs()
            )
        })?
        .with_context(|| format!("failed to fetch {} from {}", purpose, url))?
        .error_for_status()
        .with_context(|| format!("{} request failed for {}", purpose, url))?;

    if response.url().scheme() != "https" {
        bail!(
            "{} request was redirected to a non-HTTPS URL: {}",
            purpose,
            response.url()
        );
    }

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
    loop {
        let next_chunk = timeout(REMOTE_BODY_IDLE_TIMEOUT, stream.next())
            .await
            .with_context(|| {
                format!(
                    "timed out waiting for {} response body from {} after {} seconds without data",
                    purpose,
                    url,
                    REMOTE_BODY_IDLE_TIMEOUT.as_secs()
                )
            })?;
        let Some(chunk) = next_chunk else {
            break;
        };
        let chunk = chunk.context("failed to read HTTP response chunk")?;
        let next_len = bytes
            .len()
            .checked_add(chunk.len())
            .context("HTTP response size overflowed while enforcing plugin download limit")?;
        if next_len > max_bytes {
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
    let mut options = tokio::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_CLOEXEC | libc::O_NONBLOCK);
    let file = options
        .open(path)
        .await
        .with_context(|| format!("failed to open {} {}", purpose, path.display()))?;
    let metadata = file
        .metadata()
        .await
        .with_context(|| format!("failed to inspect {} {}", purpose, path.display()))?;
    if !metadata.is_file() {
        bail!("{} is not a regular file: {}", purpose, path.display());
    }

    let max_bytes_u64 = u64::try_from(max_bytes).unwrap_or(u64::MAX);
    if metadata.len() > max_bytes_u64 {
        bail!(
            "{} {} exceeds maximum allowed size ({} bytes > {} bytes)",
            purpose,
            path.display(),
            metadata.len(),
            max_bytes
        );
    }

    let read_limit = max_bytes_u64.checked_add(1).context(
        "local plugin read limit is too large to enforce with a max-plus-one bounded read",
    )?;
    let initial_capacity = usize::try_from(metadata.len())
        .unwrap_or(max_bytes)
        .min(max_bytes.saturating_add(1));
    let mut bytes = Vec::with_capacity(initial_capacity);
    let mut bounded = file.take(read_limit);
    bounded
        .read_to_end(&mut bytes)
        .await
        .with_context(|| format!("failed to read {} {}", purpose, path.display()))?;
    if bytes.len() > max_bytes {
        bail!(
            "{} {} exceeds maximum allowed size (>{} bytes)",
            purpose,
            path.display(),
            max_bytes
        );
    }

    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_read_uses_a_bounded_single_file_handle() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let path = temp.path().join("artifact.bin");
        tokio::fs::write(&path, b"12345")
            .await
            .expect("write artifact");

        let bytes = read_local_bytes_with_limit(&path, 5, "artifact")
            .await
            .expect("read exact limit");
        assert_eq!(bytes, b"12345");

        let error = read_local_bytes_with_limit(&path, 4, "artifact")
            .await
            .expect_err("oversized local artifact must fail");
        assert!(error.to_string().contains("exceeds maximum allowed size"));
    }

    #[tokio::test]
    async fn local_read_rejects_non_regular_files() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let error = read_local_bytes_with_limit(temp.path(), 16, "manifest")
            .await
            .expect_err("directory must not be read as a manifest");

        assert!(error.to_string().contains("is not a regular file"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn local_read_rejects_a_fifo_without_blocking_on_open() {
        use std::{ffi::CString, os::unix::ffi::OsStrExt as _};

        let temp = tempfile::tempdir().expect("temporary directory");
        let path = temp.path().join("artifact.pipe");
        let raw_path = CString::new(path.as_os_str().as_bytes()).expect("FIFO path without NUL");
        // SAFETY: `raw_path` is a live NUL-terminated path and the mode contains
        // only normal POSIX permission bits.
        let result = unsafe { libc::mkfifo(raw_path.as_ptr(), 0o600) };
        assert_eq!(
            result,
            0,
            "create FIFO: {}",
            std::io::Error::last_os_error()
        );

        let error = timeout(
            Duration::from_secs(1),
            read_local_bytes_with_limit(&path, 16, "artifact"),
        )
        .await
        .expect("FIFO open must not block")
        .expect_err("FIFO must not be accepted as an artifact");

        assert!(error.to_string().contains("is not a regular file"));
    }
}
