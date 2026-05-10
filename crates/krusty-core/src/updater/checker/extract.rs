use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, Result};
use tracing::debug;

pub(super) fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<()> {
    debug!("Extracting {} to {}", archive.display(), dest.display());

    let extract_dir = archive
        .parent()
        .ok_or_else(|| anyhow!("Archive path has no parent: {}", archive.display()))?
        .join("krusty-extract");
    let _ = std::fs::remove_dir_all(&extract_dir);
    std::fs::create_dir_all(&extract_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&extract_dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let archive_str = archive
        .to_str()
        .ok_or_else(|| anyhow!("Archive path is not valid UTF-8: {}", archive.display()))?;
    let extract_dir_str = extract_dir.to_str().ok_or_else(|| {
        anyhow!(
            "Extraction path is not valid UTF-8: {}",
            extract_dir.display()
        )
    })?;

    let output = Command::new("tar")
        .args(["xzf", archive_str, "-C", extract_dir_str])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("tar extraction failed: {}", stderr));
    }

    let extracted_binary = extract_dir.join("krusty");
    if !extracted_binary.exists() {
        let mut extracted_entries = String::new();
        for entry in std::fs::read_dir(&extract_dir)?.flatten() {
            if !extracted_entries.is_empty() {
                extracted_entries.push_str(", ");
            }
            extracted_entries.push_str(&entry.path().display().to_string());
        }
        debug!("Extracted contents: [{}]", extracted_entries);
        return Err(anyhow!("Binary 'krusty' not found in archive"));
    }

    let _ = std::fs::remove_file(dest);
    std::fs::copy(&extracted_binary, dest)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(dest)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(dest, perms)?;
    }

    let _ = std::fs::remove_dir_all(&extract_dir);

    debug!("Extraction complete");
    Ok(())
}

#[cfg(windows)]
pub(super) fn extract_zip(archive: &Path, dest: &Path) -> Result<()> {
    let extract_dir = archive
        .parent()
        .ok_or_else(|| anyhow!("Archive path has no parent: {}", archive.display()))?
        .join("krusty-extract");
    let _ = std::fs::remove_dir_all(&extract_dir);
    std::fs::create_dir_all(&extract_dir)?;

    let output = Command::new("powershell")
        .args([
            "-Command",
            &format!(
                "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
                archive.display(),
                extract_dir.display()
            ),
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("Zip extraction failed: {}", stderr));
    }

    let extracted = extract_dir.join("krusty.exe");
    let _ = std::fs::remove_file(dest);
    std::fs::copy(&extracted, dest)?;
    let _ = std::fs::remove_dir_all(&extract_dir);

    Ok(())
}

#[cfg(not(windows))]
pub(super) fn extract_zip(_archive: &Path, _dest: &Path) -> Result<()> {
    Err(anyhow!("Zip extraction not supported on this platform"))
}
