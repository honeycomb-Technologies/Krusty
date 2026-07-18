use anyhow::{anyhow, Context, Result};
use futures::StreamExt;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::updater::checker::apply::{discard_pending_update, write_pending_release_marker};
use crate::updater::checker::extract::{extract_tar_gz, extract_zip};
use crate::updater::checker::paths::{
    detect_platform, ensure_pending_update_dir, pending_archive_path, pending_update_path,
    pending_version_path,
};
use crate::updater::checker::types::UpdateStatus;
use crate::updater::checker::GITHUB_REPO;

const MAX_CHECKSUM_BYTES: usize = 1024;
// Release binaries are currently far smaller; this leaves ample growth room while
// bounding updater memory and disk use before checksum verification/extraction.
const MAX_RELEASE_ARCHIVE_BYTES: usize = 256 * 1024 * 1024;

pub(super) async fn download_update_release(
    version: &str,
    progress_tx: mpsc::UnboundedSender<UpdateStatus>,
) -> Result<()> {
    let _ = progress_tx.send(UpdateStatus::Downloading {
        progress: format!("Downloading v{}...", version),
    });

    let platform = detect_platform()?;
    let ext = if cfg!(windows) { "zip" } else { "tar.gz" };
    ensure_pending_update_dir()?;
    let archive_path = pending_archive_path(ext);
    remove_downloaded_archive(&archive_path).with_context(|| {
        format!(
            "failed to clear stale release archive {}",
            archive_path.display()
        )
    })?;

    let archive_name = format!("krusty-{}.{}", platform, ext);
    let url = format!(
        "https://github.com/{}/releases/download/v{}/{}",
        GITHUB_REPO, version, archive_name
    );
    let checksum_url = format!("{}.sha256", url);
    info!("Downloading: {}", url);

    let client = reqwest::Client::builder()
        .user_agent("krusty-updater")
        .timeout(std::time::Duration::from_secs(300))
        .build()?;

    let checksum_response = client
        .get(&checksum_url)
        .send()
        .await
        .context("failed to download required release checksum")?;

    if !checksum_response.status().is_success() {
        return Err(anyhow!(
            "Required checksum download failed: HTTP {}",
            checksum_response.status()
        ));
    }

    let checksum_bytes = read_checksum_body(checksum_response).await?;
    let expected_checksum = parse_published_sha256(&checksum_bytes, &archive_name)?;

    let response = client.get(&url).send().await?;

    if !response.status().is_success() {
        return Err(anyhow!("Download failed: HTTP {}", response.status()));
    }

    let bytes = read_bounded_body(response, MAX_RELEASE_ARCHIVE_BYTES, "Release archive").await?;
    info!("Downloaded {} bytes", bytes.len());

    verify_archive_sha256(&bytes, &expected_checksum)?;
    info!("Verified release checksum for {}", archive_name);

    let _ = progress_tx.send(UpdateStatus::Downloading {
        progress: "Extracting...".into(),
    });

    let archive_cleanup = scopeguard::guard(archive_path.clone(), |path| {
        if let Err(error) = remove_downloaded_archive(&path) {
            warn!(
                "Failed to remove downloaded release archive {}: {}",
                path.display(),
                error
            );
        }
    });
    std::fs::write(&archive_path, &bytes)?;
    debug!("Saved archive to: {}", archive_path.display());

    let binary_path = pending_update_path();
    let version_path = pending_version_path();
    discard_pending_update(&binary_path, &version_path)
        .context("failed to clear stale pending update artifacts")?;
    let pending_cleanup = scopeguard::guard(
        (binary_path.clone(), version_path),
        |(binary_path, version_path)| {
            if let Err(error) = discard_pending_update(&binary_path, &version_path) {
                warn!(
                    "Failed to remove incomplete pending update artifacts: {}",
                    error
                );
            }
        },
    );

    if cfg!(windows) {
        extract_zip(&archive_path, &binary_path)?;
    } else {
        extract_tar_gz(&archive_path, &binary_path)?;
    }

    if !binary_path.exists() {
        return Err(anyhow!("Extraction failed - binary not found"));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&binary_path)?.permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(&binary_path, perms)?;
    }

    let metadata = std::fs::metadata(&binary_path)?;
    info!("Extracted binary: {} bytes", metadata.len());

    remove_downloaded_archive(&archive_path).with_context(|| {
        format!(
            "failed to remove downloaded release archive {}",
            archive_path.display()
        )
    })?;
    scopeguard::ScopeGuard::into_inner(archive_cleanup);
    write_pending_release_marker(version)?;
    scopeguard::ScopeGuard::into_inner(pending_cleanup);

    let _ = progress_tx.send(UpdateStatus::Ready {
        version: version.to_string(),
    });

    info!("Update ready at: {}", binary_path.display());
    Ok(())
}

fn remove_downloaded_archive(path: &std::path::Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

async fn read_checksum_body(response: reqwest::Response) -> Result<Vec<u8>> {
    read_bounded_body(response, MAX_CHECKSUM_BYTES, "Release checksum file").await
}

async fn read_bounded_body(
    response: reqwest::Response,
    max_bytes: usize,
    description: &str,
) -> Result<Vec<u8>> {
    let content_length = response.content_length();
    if content_length.is_some_and(|length| length > max_bytes as u64) {
        return Err(anyhow!("{} exceeds {} bytes", description, max_bytes));
    }

    let initial_capacity = content_length
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or(0)
        .min(max_bytes);
    let mut body = Vec::with_capacity(initial_capacity);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.with_context(|| format!("failed while reading {}", description))?;
        let new_len = body
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| anyhow!("{} size overflow", description))?;
        if new_len > max_bytes {
            return Err(anyhow!("{} exceeds {} bytes", description, max_bytes));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn parse_published_sha256(body: &[u8], expected_archive_name: &str) -> Result<[u8; 32]> {
    if body.is_empty() {
        return Err(anyhow!("Release checksum file is empty"));
    }
    if body.len() > MAX_CHECKSUM_BYTES {
        return Err(anyhow!(
            "Release checksum file exceeds {} bytes",
            MAX_CHECKSUM_BYTES
        ));
    }
    if expected_archive_name.is_empty()
        || !expected_archive_name.is_ascii()
        || expected_archive_name.contains(['\r', '\n'])
        || expected_archive_name.contains('/')
        || expected_archive_name.contains('\\')
    {
        return Err(anyhow!("Invalid expected archive name"));
    }

    let text = std::str::from_utf8(body).context("Release checksum file is not UTF-8")?;
    if !text.is_ascii() {
        return Err(anyhow!("Release checksum file must contain only ASCII"));
    }
    let record = text.strip_suffix('\n').unwrap_or(text);
    if record.contains(['\r', '\n']) {
        return Err(anyhow!(
            "Release checksum file must contain exactly one record"
        ));
    }
    if record.len() < 66 {
        return Err(anyhow!("Release checksum record is malformed"));
    }

    let (hex_digest, file_field) = record.split_at(64);
    let published_name = file_field
        .strip_prefix("  ")
        .ok_or_else(|| anyhow!("Release checksum record must use sha256sum format"))?;
    if published_name != expected_archive_name {
        return Err(anyhow!(
            "Release checksum names '{}', expected '{}'",
            published_name,
            expected_archive_name
        ));
    }

    let mut digest = [0_u8; 32];
    for (index, pair) in hex_digest.as_bytes().chunks_exact(2).enumerate() {
        let high = decode_hex_nibble(pair[0])
            .ok_or_else(|| anyhow!("Release checksum digest is not valid hexadecimal"))?;
        let low = decode_hex_nibble(pair[1])
            .ok_or_else(|| anyhow!("Release checksum digest is not valid hexadecimal"))?;
        digest[index] = (high << 4) | low;
    }

    Ok(digest)
}

fn decode_hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn verify_archive_sha256(archive: &[u8], expected: &[u8; 32]) -> Result<()> {
    let actual = Sha256::digest(archive);
    if actual[..] != expected[..] {
        return Err(anyhow!(
            "Release archive checksum verification failed; refusing to extract"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARCHIVE_NAME: &str = "krusty-x86_64-pc-windows-msvc.zip";
    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn parses_exact_sha256sum_record() {
        let body = format!("{}  {}\n", ABC_SHA256, ARCHIVE_NAME);
        let parsed = parse_published_sha256(body.as_bytes(), ARCHIVE_NAME).expect("valid record");

        verify_archive_sha256(b"abc", &parsed).expect("matching archive");
    }

    #[test]
    fn rejects_checksum_for_a_different_archive() {
        let body = format!("{}  other.zip\n", ABC_SHA256);
        let error = parse_published_sha256(body.as_bytes(), ARCHIVE_NAME)
            .expect_err("mismatched name must fail");

        assert!(error.to_string().contains("expected"));
    }

    #[test]
    fn rejects_multiple_checksum_records_and_trailing_whitespace() {
        let multiple = format!(
            "{}  {}\n{}  other.zip\n",
            ABC_SHA256, ARCHIVE_NAME, ABC_SHA256
        );
        assert!(parse_published_sha256(multiple.as_bytes(), ARCHIVE_NAME).is_err());

        let trailing_space = format!("{}  {} \n", ABC_SHA256, ARCHIVE_NAME);
        assert!(parse_published_sha256(trailing_space.as_bytes(), ARCHIVE_NAME).is_err());
    }

    #[test]
    fn rejects_malformed_digest_and_oversized_checksum() {
        let malformed = format!("{}  {}\n", "z".repeat(64), ARCHIVE_NAME);
        assert!(parse_published_sha256(malformed.as_bytes(), ARCHIVE_NAME).is_err());

        let non_ascii = format!("{}é  {}\n", "a".repeat(63), ARCHIVE_NAME);
        assert!(parse_published_sha256(non_ascii.as_bytes(), ARCHIVE_NAME).is_err());

        let oversized = vec![b'a'; MAX_CHECKSUM_BYTES + 1];
        assert!(parse_published_sha256(&oversized, ARCHIVE_NAME).is_err());
    }

    #[test]
    fn rejects_archive_with_wrong_digest() {
        let body = format!("{}  {}\n", ABC_SHA256, ARCHIVE_NAME);
        let parsed = parse_published_sha256(body.as_bytes(), ARCHIVE_NAME).expect("valid record");
        let error = verify_archive_sha256(b"different", &parsed)
            .expect_err("checksum mismatch must fail closed");

        assert!(error.to_string().contains("refusing to extract"));
    }

    #[test]
    fn stale_archive_cleanup_is_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let archive = dir.path().join("krusty-download.zip");
        std::fs::write(&archive, b"stale").expect("write stale archive");

        remove_downloaded_archive(&archive).expect("remove stale archive");
        remove_downloaded_archive(&archive).expect("missing archive remains clean");

        assert!(!archive.exists());
    }
}
