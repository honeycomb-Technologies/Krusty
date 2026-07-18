use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use tracing::debug;

// A release binary should remain well below this ceiling. The limit prevents a
// highly-compressible ZIP entry from expanding without bound on the local disk.
const MAX_EXTRACTED_BINARY_BYTES: u64 = 512 * 1024 * 1024;

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

pub(super) fn extract_zip(archive: &Path, dest: &Path) -> Result<()> {
    debug!(
        "Extracting exact krusty.exe entry from {}",
        archive.display()
    );
    let staged = dest.with_extension("extracting");
    let _ = std::fs::remove_file(&staged);

    if let Err(error) = extract_exact_windows_binary(archive, &staged) {
        let _ = std::fs::remove_file(&staged);
        return Err(error);
    }

    let _ = std::fs::remove_file(dest);
    if let Err(error) = std::fs::rename(&staged, dest) {
        let _ = std::fs::remove_file(&staged);
        return Err(error)
            .with_context(|| format!("failed to install extracted binary at {}", dest.display()));
    }

    Ok(())
}

fn extract_exact_windows_binary(archive: &Path, staged: &Path) -> Result<()> {
    let file = File::open(archive)
        .with_context(|| format!("failed to open release archive {}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(file).context("release ZIP is invalid")?;
    if zip.len() != 1 {
        return Err(anyhow!(
            "Release ZIP must contain exactly one krusty.exe entry, found {}",
            zip.len()
        ));
    }

    let mut entry = zip
        .by_index(0)
        .context("failed to read release ZIP entry")?;
    let enclosed_name = entry
        .enclosed_name()
        .ok_or_else(|| anyhow!("Release ZIP entry has an unsafe path"))?;
    if entry.name() != "krusty.exe" || enclosed_name != Path::new("krusty.exe") {
        return Err(anyhow!(
            "Release ZIP entry must be exactly 'krusty.exe', found '{}'",
            entry.name()
        ));
    }
    if !entry.is_file() {
        return Err(anyhow!("Release ZIP entry is not a regular file"));
    }
    if entry
        .unix_mode()
        .is_some_and(|mode| mode & 0o170000 != 0o100000)
    {
        return Err(anyhow!("Release ZIP entry has an unsupported file type"));
    }
    if entry.size() > MAX_EXTRACTED_BINARY_BYTES {
        return Err(anyhow!(
            "Extracted release binary exceeds {} bytes",
            MAX_EXTRACTED_BINARY_BYTES
        ));
    }

    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(staged)
        .with_context(|| format!("failed to stage extracted binary at {}", staged.display()))?;
    let copied = std::io::copy(
        &mut (&mut entry).take(MAX_EXTRACTED_BINARY_BYTES + 1),
        &mut output,
    )
    .context("failed to extract krusty.exe")?;
    if copied > MAX_EXTRACTED_BINARY_BYTES {
        return Err(anyhow!(
            "Extracted release binary exceeds {} bytes",
            MAX_EXTRACTED_BINARY_BYTES
        ));
    }
    output
        .sync_all()
        .context("failed to sync extracted binary")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use zip::write::SimpleFileOptions;

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = File::create(path).expect("create zip");
        let mut writer = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default().unix_permissions(0o755);
        for (name, contents) in entries {
            writer.start_file(*name, options).expect("start entry");
            writer.write_all(contents).expect("write entry");
        }
        writer.finish().expect("finish zip");
    }

    #[test]
    fn extracts_only_exact_windows_binary_entry() {
        let dir = tempfile::tempdir().expect("tempdir");
        let archive = dir.path().join("release.zip");
        let dest = dir.path().join("pending.exe");
        write_zip(&archive, &[("krusty.exe", b"binary")]);

        extract_zip(&archive, &dest).expect("extract exact entry");

        assert_eq!(std::fs::read(dest).expect("read binary"), b"binary");
    }

    #[test]
    fn rejects_zip_with_unexpected_or_traversing_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        let extra_archive = dir.path().join("extra.zip");
        let traversal_archive = dir.path().join("traversal.zip");
        let dest = dir.path().join("pending.exe");
        write_zip(
            &extra_archive,
            &[("krusty.exe", b"binary"), ("extra.txt", b"unexpected")],
        );
        write_zip(&traversal_archive, &[("../krusty.exe", b"binary")]);

        assert!(extract_zip(&extra_archive, &dest).is_err());
        assert!(!dest.exists());
        assert!(!dest.with_extension("extracting").exists());
        assert!(extract_zip(&traversal_archive, &dest).is_err());
        assert!(!dest.exists());
        assert!(!dest.with_extension("extracting").exists());
    }
}
