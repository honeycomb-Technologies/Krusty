//! Update checker - supports both dev (git) and release (GitHub) modes

use anyhow::{anyhow, Result};
use std::path::PathBuf;
use std::process::Command;
use tokio::sync::mpsc;

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
    /// Download/build complete, ready to apply
    Ready { new_binary: PathBuf, version: String },
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

/// Detect if running in dev mode (from git repo) or release mode (installed binary)
pub fn is_dev_mode() -> bool {
    detect_repo_path().is_some()
}

/// Check for updates - automatically chooses dev or release mode
pub async fn check_for_updates() -> Result<Option<UpdateInfo>> {
    if let Some(repo_path) = detect_repo_path() {
        check_for_updates_dev(&repo_path)
    } else {
        check_for_updates_release().await
    }
}

/// Check for updates via GitHub releases API
async fn check_for_updates_release() -> Result<Option<UpdateInfo>> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", GITHUB_REPO);

    let client = reqwest::Client::builder()
        .user_agent("krusty-updater")
        .build()?;

    let response = client.get(&url).send().await?;

    if !response.status().is_success() {
        return Err(anyhow!("Failed to check for updates: {}", response.status()));
    }

    let release: serde_json::Value = response.json().await?;

    let tag_name = release["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow!("No tag_name in release"))?;

    // Strip 'v' prefix if present
    let new_version = tag_name.strip_prefix('v').unwrap_or(tag_name);

    // Compare versions
    if new_version == VERSION {
        return Ok(None);
    }

    // Simple version comparison (semver)
    if !is_newer_version(new_version, VERSION) {
        return Ok(None);
    }

    let release_notes = release["body"]
        .as_str()
        .unwrap_or("")
        .lines()
        .next()
        .unwrap_or("New version available")
        .to_string();

    Ok(Some(UpdateInfo {
        current_version: VERSION.to_string(),
        new_version: new_version.to_string(),
        release_notes,
        is_dev_mode: false,
    }))
}

/// Check for updates via git (dev mode)
fn check_for_updates_dev(repo_path: &PathBuf) -> Result<Option<UpdateInfo>> {
    // Fetch from origin (quiet)
    let fetch_status = Command::new("git")
        .args(["fetch", "origin", "main", "--quiet"])
        .current_dir(repo_path)
        .status()?;

    if !fetch_status.success() {
        return Err(anyhow!("Failed to fetch from origin"));
    }

    // Get current HEAD
    let current = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(repo_path)
        .output()?;
    let current_commit = String::from_utf8_lossy(&current.stdout).trim().to_string();

    // Get origin/main HEAD
    let remote = Command::new("git")
        .args(["rev-parse", "--short", "origin/main"])
        .current_dir(repo_path)
        .output()?;
    let new_commit = String::from_utf8_lossy(&remote.stdout).trim().to_string();

    // If same, we're up to date
    if current_commit == new_commit {
        return Ok(None);
    }

    // Get latest commit message
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

/// Download and prepare update - automatically chooses dev or release mode
pub async fn download_update(
    info: &UpdateInfo,
    progress_tx: mpsc::UnboundedSender<UpdateStatus>,
) -> Result<PathBuf> {
    if info.is_dev_mode {
        let repo_path = detect_repo_path().ok_or_else(|| anyhow!("No repo path for dev mode"))?;
        download_update_dev(repo_path, progress_tx).await
    } else {
        download_update_release(&info.new_version, progress_tx).await
    }
}

/// Download update from GitHub releases
async fn download_update_release(
    version: &str,
    progress_tx: mpsc::UnboundedSender<UpdateStatus>,
) -> Result<PathBuf> {
    progress_tx.send(UpdateStatus::Downloading {
        progress: format!("Downloading v{}...", version),
    })?;

    // Detect platform
    let platform = detect_platform()?;
    let ext = if cfg!(windows) { "zip" } else { "tar.gz" };

    let url = format!(
        "https://github.com/{}/releases/download/v{}/krusty-{}.{}",
        GITHUB_REPO, version, platform, ext
    );

    let client = reqwest::Client::builder()
        .user_agent("krusty-updater")
        .build()?;

    let response = client.get(&url).send().await?;

    if !response.status().is_success() {
        return Err(anyhow!("Failed to download update: {}", response.status()));
    }

    let bytes = response.bytes().await?;

    progress_tx.send(UpdateStatus::Downloading {
        progress: "Extracting...".into(),
    })?;

    // Extract to temp directory
    let temp_dir = std::env::temp_dir();
    let archive_path = temp_dir.join(format!("krusty-update.{}", ext));
    let binary_path = temp_dir.join("krusty-update");

    std::fs::write(&archive_path, &bytes)?;

    // Extract binary
    if cfg!(windows) {
        extract_zip(&archive_path, &binary_path)?;
    } else {
        extract_tar_gz(&archive_path, &binary_path)?;
    }

    // Cleanup archive
    let _ = std::fs::remove_file(&archive_path);

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&binary_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&binary_path, perms)?;
    }

    progress_tx.send(UpdateStatus::Ready {
        new_binary: binary_path.clone(),
        version: version.to_string(),
    })?;

    Ok(binary_path)
}

/// Build update from git (dev mode)
async fn download_update_dev(
    repo_path: PathBuf,
    progress_tx: mpsc::UnboundedSender<UpdateStatus>,
) -> Result<PathBuf> {
    progress_tx.send(UpdateStatus::Downloading {
        progress: "Pulling latest changes...".into(),
    })?;

    let pull = tokio::process::Command::new("git")
        .args(["pull", "origin", "main"])
        .current_dir(&repo_path)
        .output()
        .await?;

    if !pull.status.success() {
        let err = String::from_utf8_lossy(&pull.stderr);
        return Err(anyhow!("Git pull failed: {}", err));
    }

    progress_tx.send(UpdateStatus::Downloading {
        progress: "Building release binary...".into(),
    })?;

    let build = tokio::process::Command::new("cargo")
        .args(["build", "--release", "-p", "krusty"])
        .current_dir(&repo_path)
        .output()
        .await?;

    if !build.status.success() {
        let err = String::from_utf8_lossy(&build.stderr);
        return Err(anyhow!("Cargo build failed: {}", err));
    }

    progress_tx.send(UpdateStatus::Downloading {
        progress: "Preparing update...".into(),
    })?;

    let source = repo_path.join("target/release/krusty");
    let temp_dir = std::env::temp_dir();
    let dest = temp_dir.join("krusty-update");

    std::fs::copy(&source, &dest)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dest)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dest, perms)?;
    }

    // Get new version from git
    let version = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(&repo_path)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    progress_tx.send(UpdateStatus::Ready {
        new_binary: dest.clone(),
        version,
    })?;

    Ok(dest)
}

/// Apply update by replacing current binary
pub fn apply_update(new_binary: &PathBuf) -> Result<()> {
    let current_exe = std::env::current_exe()?;

    // On Windows, we can't replace a running binary directly
    // On Unix, we can rename and replace
    #[cfg(unix)]
    {
        let backup = current_exe.with_extension("old");

        // Move current to backup
        std::fs::rename(&current_exe, &backup)?;

        // Move new to current location
        std::fs::copy(new_binary, &current_exe)?;

        // Set permissions
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&current_exe)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&current_exe, perms)?;

        // Remove backup
        let _ = std::fs::remove_file(&backup);
    }

    #[cfg(windows)]
    {
        // Windows: need to use a different approach (rename current, copy new)
        let backup = current_exe.with_extension("exe.old");
        std::fs::rename(&current_exe, &backup)?;
        std::fs::copy(new_binary, &current_exe)?;
    }

    Ok(())
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

/// Extract tar.gz archive
fn extract_tar_gz(archive: &PathBuf, dest: &PathBuf) -> Result<()> {
    use std::process::Command;

    let output = Command::new("tar")
        .args(["xzf", archive.to_str().unwrap(), "-O"])
        .output()?;

    if !output.status.success() {
        return Err(anyhow!("Failed to extract archive"));
    }

    std::fs::write(dest, output.stdout)?;
    Ok(())
}

/// Extract zip archive (Windows)
#[cfg(windows)]
fn extract_zip(archive: &PathBuf, dest: &PathBuf) -> Result<()> {
    use std::process::Command;

    let temp_dir = archive.parent().unwrap();
    let output = Command::new("powershell")
        .args([
            "-Command",
            &format!(
                "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
                archive.display(),
                temp_dir.display()
            ),
        ])
        .output()?;

    if !output.status.success() {
        return Err(anyhow!("Failed to extract archive"));
    }

    // Find and move the binary
    let extracted = temp_dir.join("krusty.exe");
    std::fs::rename(&extracted, dest)?;
    Ok(())
}

#[cfg(not(windows))]
fn extract_zip(_archive: &PathBuf, _dest: &PathBuf) -> Result<()> {
    Err(anyhow!("Zip extraction not supported on this platform"))
}
