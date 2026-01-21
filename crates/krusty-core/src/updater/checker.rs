//! Update checker - supports both dev (git) and release (GitHub) modes

use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use tokio::sync::mpsc;
use tracing::{debug, info};

/// Current version from Cargo.toml
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// GitHub repo for releases
const GITHUB_REPO: &str = "BurgessTG/Krusty";

/// Update status
#[derive(Debug, Clone, PartialEq)]
pub enum UpdateStatus {
    /// Currently checking for updates
    Checking,
    /// No updates available
    UpToDate,
    /// Update available
    Available(UpdateInfo),
    /// Downloading/building update
    Downloading { progress: String },
    /// Download/build complete, ready to apply on restart
    Ready { version: String },
    /// Error occurred
    Error(String),
}

/// Information about an available update
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateInfo {
    /// Current version
    pub current_version: String,
    /// New version available
    pub new_version: String,
    /// Release notes / commit message
    pub release_notes: String,
    /// Whether this is a dev (git) or release (GitHub) update
    pub is_dev_mode: bool,
}

/// Path to the pending update binary
pub fn pending_update_path() -> PathBuf {
    std::env::temp_dir().join("krusty-pending-update")
}

/// Check if there's a pending update ready to apply
pub fn has_pending_update() -> bool {
    pending_update_path().exists()
}

/// Detect if running in dev mode (from git repo) or release mode (installed binary)
pub fn is_dev_mode() -> bool {
    detect_repo_path().is_some()
}

/// Check for updates - automatically chooses dev or release mode
pub async fn check_for_updates() -> Result<Option<UpdateInfo>> {
    info!("Checking for updates (current version: {})", VERSION);

    if let Some(repo_path) = detect_repo_path() {
        debug!("Dev mode detected, checking git for updates");
        check_for_updates_dev(&repo_path)
    } else {
        debug!("Release mode, checking GitHub releases");
        check_for_updates_release().await
    }
}

/// Check for updates via GitHub releases API
async fn check_for_updates_release() -> Result<Option<UpdateInfo>> {
    let url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        GITHUB_REPO
    );
    debug!("Fetching: {}", url);

    let client = reqwest::Client::builder()
        .user_agent("krusty-updater")
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let response = client.get(&url).send().await?;

    if !response.status().is_success() {
        return Err(anyhow!("GitHub API returned: {}", response.status()));
    }

    let release: serde_json::Value = response.json().await?;

    let tag_name = release["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow!("No tag_name in release"))?;

    // Strip 'v' prefix if present
    let new_version = tag_name.strip_prefix('v').unwrap_or(tag_name);
    debug!("Latest release: {} (current: {})", new_version, VERSION);

    // Compare versions
    if new_version == VERSION {
        info!("Already up to date");
        return Ok(None);
    }

    // Simple version comparison (semver)
    if !is_newer_version(new_version, VERSION) {
        info!("Current version is newer than release");
        return Ok(None);
    }

    let release_notes = release["body"]
        .as_str()
        .unwrap_or("")
        .lines()
        .next()
        .unwrap_or("New version available")
        .to_string();

    info!("Update available: {} -> {}", VERSION, new_version);

    Ok(Some(UpdateInfo {
        current_version: VERSION.to_string(),
        new_version: new_version.to_string(),
        release_notes,
        is_dev_mode: false,
    }))
}

/// Check for updates via git (dev mode)
fn check_for_updates_dev(repo_path: &PathBuf) -> Result<Option<UpdateInfo>> {
    debug!("Fetching from origin...");

    let fetch_status = Command::new("git")
        .args(["fetch", "origin", "main", "--quiet"])
        .current_dir(repo_path)
        .status()?;

    if !fetch_status.success() {
        return Err(anyhow!("Failed to fetch from origin"));
    }

    let current = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(repo_path)
        .output()?;
    let current_commit = String::from_utf8_lossy(&current.stdout).trim().to_string();

    let remote = Command::new("git")
        .args(["rev-parse", "--short", "origin/main"])
        .current_dir(repo_path)
        .output()?;
    let new_commit = String::from_utf8_lossy(&remote.stdout).trim().to_string();

    debug!("Current: {}, Remote: {}", current_commit, new_commit);

    if current_commit == new_commit {
        return Ok(None);
    }

    let msg = Command::new("git")
        .args(["log", "-1", "--format=%s", "origin/main"])
        .current_dir(repo_path)
        .output()?;
    let commit_message = String::from_utf8_lossy(&msg.stdout).trim().to_string();

    Ok(Some(UpdateInfo {
        current_version: current_commit,
        new_version: new_commit,
        release_notes: commit_message,
        is_dev_mode: true,
    }))
}

/// Download and prepare update - saves to temp location for later apply
pub async fn download_update(
    info: &UpdateInfo,
    progress_tx: mpsc::UnboundedSender<UpdateStatus>,
) -> Result<()> {
    if info.is_dev_mode {
        let repo_path = detect_repo_path().ok_or_else(|| anyhow!("No repo path for dev mode"))?;
        download_update_dev(repo_path, &info.new_version, progress_tx).await
    } else {
        download_update_release(&info.new_version, progress_tx).await
    }
}

/// Download update from GitHub releases
async fn download_update_release(
    version: &str,
    progress_tx: mpsc::UnboundedSender<UpdateStatus>,
) -> Result<()> {
    let _ = progress_tx.send(UpdateStatus::Downloading {
        progress: format!("Downloading v{}...", version),
    });

    let platform = detect_platform()?;
    let ext = if cfg!(windows) { "zip" } else { "tar.gz" };

    let url = format!(
        "https://github.com/{}/releases/download/v{}/krusty-{}.{}",
        GITHUB_REPO, version, platform, ext
    );
    info!("Downloading: {}", url);

    let client = reqwest::Client::builder()
        .user_agent("krusty-updater")
        .timeout(std::time::Duration::from_secs(300))
        .build()?;

    let response = client.get(&url).send().await?;

    if !response.status().is_success() {
        return Err(anyhow!("Download failed: HTTP {}", response.status()));
    }

    let bytes = response.bytes().await?;
    info!("Downloaded {} bytes", bytes.len());

    let _ = progress_tx.send(UpdateStatus::Downloading {
        progress: "Extracting...".into(),
    });

    // Save archive to temp
    let temp_dir = std::env::temp_dir();
    let archive_path = temp_dir.join(format!("krusty-download.{}", ext));
    std::fs::write(&archive_path, &bytes)?;
    debug!("Saved archive to: {}", archive_path.display());

    // Extract binary
    let binary_path = pending_update_path();

    if cfg!(windows) {
        extract_zip(&archive_path, &binary_path)?;
    } else {
        extract_tar_gz(&archive_path, &binary_path)?;
    }

    // Verify binary exists and is executable
    if !binary_path.exists() {
        return Err(anyhow!("Extraction failed - binary not found"));
    }

    let metadata = std::fs::metadata(&binary_path)?;
    info!("Extracted binary: {} bytes", metadata.len());

    // Cleanup archive
    let _ = std::fs::remove_file(&archive_path);

    let _ = progress_tx.send(UpdateStatus::Ready {
        version: version.to_string(),
    });

    info!("Update ready at: {}", binary_path.display());
    Ok(())
}

/// Build update from git (dev mode)
async fn download_update_dev(
    repo_path: PathBuf,
    version: &str,
    progress_tx: mpsc::UnboundedSender<UpdateStatus>,
) -> Result<()> {
    let _ = progress_tx.send(UpdateStatus::Downloading {
        progress: "Pulling latest changes...".into(),
    });

    let pull = tokio::process::Command::new("git")
        .args(["pull", "origin", "main"])
        .current_dir(&repo_path)
        .output()
        .await?;

    if !pull.status.success() {
        let err = String::from_utf8_lossy(&pull.stderr);
        return Err(anyhow!("Git pull failed: {}", err));
    }

    let _ = progress_tx.send(UpdateStatus::Downloading {
        progress: "Building release binary...".into(),
    });

    let build = tokio::process::Command::new("cargo")
        .args(["build", "--release", "-p", "krusty"])
        .current_dir(&repo_path)
        .output()
        .await?;

    if !build.status.success() {
        let err = String::from_utf8_lossy(&build.stderr);
        return Err(anyhow!("Cargo build failed: {}", err));
    }

    let _ = progress_tx.send(UpdateStatus::Downloading {
        progress: "Preparing update...".into(),
    });

    let source = repo_path.join("target/release/krusty");
    let dest = pending_update_path();

    std::fs::copy(&source, &dest)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dest)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dest, perms)?;
    }

    let _ = progress_tx.send(UpdateStatus::Ready {
        version: version.to_string(),
    });

    Ok(())
}

/// Apply pending update by replacing current binary
/// MUST be called early in startup, before TUI runs
pub fn apply_pending_update() -> Result<Option<String>> {
    let pending = pending_update_path();

    if !pending.exists() {
        return Ok(None);
    }

    info!("Found pending update at: {}", pending.display());

    let current_exe = std::env::current_exe()?;
    info!("Current binary: {}", current_exe.display());

    #[cfg(unix)]
    {
        let backup = current_exe.with_extension("old");

        // Rename current binary to backup
        debug!("Renaming current to: {}", backup.display());
        std::fs::rename(&current_exe, &backup)?;

        // Copy new binary to current location
        debug!("Copying new binary to: {}", current_exe.display());
        std::fs::copy(&pending, &current_exe)?;

        // Set executable permissions
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&current_exe)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&current_exe, perms)?;

        // Remove backup and pending
        let _ = std::fs::remove_file(&backup);
        let _ = std::fs::remove_file(&pending);

        info!("Update applied successfully");
    }

    #[cfg(windows)]
    {
        let backup = current_exe.with_extension("exe.old");
        std::fs::rename(&current_exe, &backup)?;
        std::fs::copy(&pending, &current_exe)?;
        let _ = std::fs::remove_file(&pending);
        info!("Update applied successfully");
    }

    // Try to read version from the new binary (optional)
    Ok(Some("latest".to_string()))
}

/// Clean up any pending update (if user wants to cancel)
pub fn cleanup_pending_update() {
    let pending = pending_update_path();
    if pending.exists() {
        let _ = std::fs::remove_file(&pending);
        info!("Cleaned up pending update");
    }
}

/// Detect repo path (for dev mode)
pub fn detect_repo_path() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            if parent.ends_with("release") {
                if let Some(target) = parent.parent() {
                    if target.ends_with("target") {
                        if let Some(repo) = target.parent() {
                            if repo.join("Cargo.toml").exists() {
                                return Some(repo.to_path_buf());
                            }
                        }
                    }
                }
            }
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("Cargo.toml").exists() && cwd.join("crates/krusty-cli").exists() {
            return Some(cwd);
        }
    }

    None
}

/// Detect platform string for download
#[allow(clippy::unnecessary_wraps)] // Result is needed for unsupported platforms
fn detect_platform() -> Result<&'static str> {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return Ok("x86_64-unknown-linux-gnu");

    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return Ok("aarch64-unknown-linux-gnu");

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return Ok("x86_64-apple-darwin");

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return Ok("aarch64-apple-darwin");

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return Ok("x86_64-pc-windows-msvc");

    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "windows", target_arch = "x86_64"),
    )))]
    return Err(anyhow!("Unsupported platform"));
}

/// Compare semver versions (returns true if new > current)
fn is_newer_version(new: &str, current: &str) -> bool {
    let parse = |s: &str| -> (u32, u32, u32) {
        let parts: Vec<u32> = s.split('.').filter_map(|p| p.parse().ok()).collect();
        (
            parts.first().copied().unwrap_or(0),
            parts.get(1).copied().unwrap_or(0),
            parts.get(2).copied().unwrap_or(0),
        )
    };

    let new_v = parse(new);
    let curr_v = parse(current);

    new_v > curr_v
}

/// Extract tar.gz archive properly
fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<()> {
    debug!("Extracting {} to {}", archive.display(), dest.display());

    // Create a temp extraction directory
    let extract_dir = std::env::temp_dir().join("krusty-extract");
    let _ = std::fs::remove_dir_all(&extract_dir);
    std::fs::create_dir_all(&extract_dir)?;

    // Extract archive to temp directory
    let output = Command::new("tar")
        .args([
            "xzf",
            archive.to_str().unwrap(),
            "-C",
            extract_dir.to_str().unwrap(),
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("tar extraction failed: {}", stderr));
    }

    // Find the krusty binary (should be at root of archive)
    let extracted_binary = extract_dir.join("krusty");
    if !extracted_binary.exists() {
        // Maybe it's in a subdirectory?
        let entries: Vec<_> = std::fs::read_dir(&extract_dir)?
            .filter_map(|e| e.ok())
            .collect();
        debug!(
            "Extracted contents: {:?}",
            entries.iter().map(|e| e.path()).collect::<Vec<_>>()
        );
        return Err(anyhow!("Binary 'krusty' not found in archive"));
    }

    // Move to destination
    std::fs::copy(&extracted_binary, dest)?;

    // Set executable permissions
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(dest)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(dest, perms)?;
    }

    // Cleanup
    let _ = std::fs::remove_dir_all(&extract_dir);

    debug!("Extraction complete");
    Ok(())
}

/// Extract zip archive (Windows)
#[cfg(windows)]
fn extract_zip(archive: &PathBuf, dest: &PathBuf) -> Result<()> {
    let extract_dir = std::env::temp_dir().join("krusty-extract");
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
    std::fs::copy(&extracted, dest)?;
    let _ = std::fs::remove_dir_all(&extract_dir);

    Ok(())
}

#[cfg(not(windows))]
fn extract_zip(_archive: &PathBuf, _dest: &PathBuf) -> Result<()> {
    Err(anyhow!("Zip extraction not supported on this platform"))
}
