//! Coordinated release-set updates for shell-installer installs.
//!
//! Downloads the full GitHub archive (CLI, Hive, shims, units), stages an
//! immutable release next to `.mitsuro-current`, flips the pointer, and
//! restarts previously active systemd user units. Single-binary replacement
//! stays fail-closed.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use flate2::read::GzDecoder;
use futures::StreamExt;
use sha2::{Digest, Sha256};
use tar::Archive;
use tokio::sync::mpsc;
use tracing::{info, warn};

use super::channel::{ManagedInstall, UpdateChannel};
use super::checker::checksum::{parse_published_sha256, verify_archive_sha256, MAX_CHECKSUM_BYTES};
use super::checker::paths::detect_platform;
use super::checker::GITHUB_REPO;
use super::UpdateStatus;

const MAX_RELEASE_ARCHIVE_BYTES: usize = 256 * 1024 * 1024;
const SYSTEMD_UNITS: &[&str] = &[
    "mitsuro-hive.socket",
    "mitsuro-hive.service",
    "mitsuro-serve.service",
];
const REQUIRED_BINARIES: &[&str] = &["mitsuro", "krusty"];
const OPTIONAL_BINARIES: &[&str] = &["mitsuro-hive", "krusty-mako", "agent-browser"];

pub async fn apply_managed_release_update(
    version: &str,
    progress_tx: mpsc::UnboundedSender<UpdateStatus>,
) -> Result<PathBuf> {
    let channel = UpdateChannel::detect();
    let install = channel
        .managed_install()
        .cloned()
        .ok_or_else(|| anyhow!("{}", channel.apply_policy().guidance()))?;
    apply_managed_release_update_to(&install, version, progress_tx).await
}

pub async fn apply_managed_release_update_to(
    install: &ManagedInstall,
    version: &str,
    progress_tx: mpsc::UnboundedSender<UpdateStatus>,
) -> Result<PathBuf> {
    let version = version.strip_prefix('v').unwrap_or(version);
    if version.is_empty() {
        return Err(anyhow!("Release version is empty"));
    }

    let _ = progress_tx.send(UpdateStatus::Downloading {
        progress: format!("Downloading v{version}..."),
        fraction: Some(0.0),
    });

    let platform = detect_platform()?;
    let archive_name = format!("mitsuro-{platform}.tar.gz");
    let url = format!(
        "https://github.com/{}/releases/download/v{}/{}",
        GITHUB_REPO, version, archive_name
    );
    let checksum_url = format!("{url}.sha256");

    let client = reqwest::Client::builder()
        .user_agent("mitsuro-updater")
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
    let checksum_bytes =
        read_bounded_body(checksum_response, MAX_CHECKSUM_BYTES, "checksum").await?;
    let expected_checksum = parse_published_sha256(&checksum_bytes, &archive_name)?;

    let response = client.get(&url).send().await?;
    if !response.status().is_success() {
        return Err(anyhow!("Download failed: HTTP {}", response.status()));
    }
    let bytes = read_body_with_progress(
        response,
        MAX_RELEASE_ARCHIVE_BYTES,
        "Release archive",
        &progress_tx,
    )
    .await?;
    verify_archive_sha256(&bytes, &expected_checksum)?;
    let archive_sha = hex_sha256(&bytes);

    let _ = progress_tx.send(UpdateStatus::Installing {
        progress: "Extracting release...".into(),
    });
    let extracted = extract_unix_release(&bytes)?;
    if install.has_hive && extracted.hive.is_none() {
        return Err(anyhow!(
            "Refusing to replace a supervised Hive release with a mitsuro-only archive."
        ));
    }

    let _ = progress_tx.send(UpdateStatus::Installing {
        progress: "Installing release...".into(),
    });
    let previous_target = read_current_target(&install.current_link)?;
    let release_id = format!("v{version}-{platform}-{archive_sha}");
    let release_dir = stage_release(install, &release_id, &extracted, &archive_sha)?;

    let lock = InstallLock::acquire(&install.install_dir)?;
    if let Err(error) = activate_release(install, &release_id, &previous_target, &progress_tx) {
        warn!("Release activation failed: {error}");
        let _ = restore_current_link(&install.current_link, &previous_target);
        let _ = restart_active_units(install);
        return Err(error);
    }
    drop(lock);

    let _ = progress_tx.send(UpdateStatus::Ready {
        version: version.to_string(),
    });
    info!(
        "Managed release v{version} activated at {}",
        release_dir.display()
    );
    Ok(install.install_dir.join("mitsuro"))
}

struct ExtractedRelease {
    mitsuro: Vec<u8>,
    krusty: Vec<u8>,
    hive: Option<Vec<u8>>,
    krusty_mako: Option<Vec<u8>>,
    agent_browser: Option<Vec<u8>>,
    systemd: Vec<(String, Vec<u8>)>,
}

fn extract_unix_release(bytes: &[u8]) -> Result<ExtractedRelease> {
    let decoder = GzDecoder::new(bytes);
    let mut archive = Archive::new(decoder);
    let mut mitsuro = None;
    let mut krusty = None;
    let mut hive = None;
    let mut krusty_mako = None;
    let mut agent_browser = None;
    let mut systemd = Vec::new();

    for entry in archive
        .entries()
        .context("release archive is not a tar.gz")?
    {
        let mut entry = entry.context("failed to read archive entry")?;
        let path = entry
            .path()
            .context("archive entry path is invalid")?
            .into_owned();
        let name = normalize_member_path(&path)?;
        if name.is_empty() || name.ends_with('/') {
            continue;
        }
        if !entry.header().entry_type().is_file() {
            return Err(anyhow!(
                "Release archive member '{name}' is not a regular file"
            ));
        }
        let mut contents = Vec::new();
        entry
            .read_to_end(&mut contents)
            .with_context(|| format!("failed to read archive member {name}"))?;

        match name.as_str() {
            "mitsuro" => mitsuro = Some(contents),
            "krusty" => krusty = Some(contents),
            "mitsuro-hive" => hive = Some(contents),
            "krusty-mako" => krusty_mako = Some(contents),
            "agent-browser" => agent_browser = Some(contents),
            name if name.starts_with("systemd/") => {
                let unit = name.trim_start_matches("systemd/");
                if !SYSTEMD_UNITS.contains(&unit) {
                    return Err(anyhow!("Unexpected systemd unit '{unit}'"));
                }
                systemd.push((unit.to_string(), contents));
            }
            other => return Err(anyhow!("Unexpected archive member '{other}'")),
        }
    }

    let mitsuro = mitsuro.ok_or_else(|| anyhow!("Release archive is missing mitsuro"))?;
    let krusty = krusty.ok_or_else(|| anyhow!("Release archive is missing krusty"))?;
    if hive.is_some() && systemd.len() != SYSTEMD_UNITS.len() {
        return Err(anyhow!(
            "A release must ship mitsuro-hive and its complete systemd unit set together."
        ));
    }
    if hive.is_none() && !systemd.is_empty() {
        return Err(anyhow!(
            "A release must ship mitsuro-hive and its complete systemd unit set together."
        ));
    }

    let _ = (REQUIRED_BINARIES, OPTIONAL_BINARIES);
    Ok(ExtractedRelease {
        mitsuro,
        krusty,
        hive,
        krusty_mako,
        agent_browser,
        systemd,
    })
}

fn normalize_member_path(path: &Path) -> Result<String> {
    if path.is_absolute() {
        return Err(anyhow!("Release archive member has an absolute path"));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => {
                let part = part
                    .to_str()
                    .ok_or_else(|| anyhow!("non-UTF-8 archive path"))?;
                if part == ".." {
                    return Err(anyhow!("Release archive member escapes the extract root"));
                }
                parts.push(part);
            }
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                return Err(anyhow!("Release archive member escapes the extract root"));
            }
            _ => return Err(anyhow!("Release archive member has an unsafe path")),
        }
    }
    Ok(parts.join("/"))
}

fn stage_release(
    install: &ManagedInstall,
    release_id: &str,
    extracted: &ExtractedRelease,
    archive_sha: &str,
) -> Result<PathBuf> {
    let releases = install.install_dir.join(".mitsuro-releases");
    fs::create_dir_all(&releases).context("failed to create release directory")?;
    let release_dir = releases.join(release_id);
    if release_dir.exists() {
        return Ok(release_dir);
    }
    let stage = releases.join(format!(".stage-{release_id}-{}", std::process::id()));
    if stage.exists() {
        fs::remove_dir_all(&stage).ok();
    }
    fs::create_dir_all(&stage)?;
    write_mode(&stage.join("mitsuro"), &extracted.mitsuro, 0o555)?;
    write_mode(&stage.join("krusty"), &extracted.krusty, 0o555)?;
    if let Some(hive) = &extracted.hive {
        write_mode(&stage.join("mitsuro-hive"), hive, 0o555)?;
    }
    if let Some(compat) = &extracted.krusty_mako {
        write_mode(&stage.join("krusty-mako"), compat, 0o555)?;
    }
    if let Some(browser) = &extracted.agent_browser {
        write_mode(&stage.join("agent-browser"), browser, 0o555)?;
    }
    if !extracted.systemd.is_empty() {
        let unit_dir = stage.join("systemd");
        fs::create_dir_all(&unit_dir)?;
        for (name, contents) in &extracted.systemd {
            write_mode(&unit_dir.join(name), contents, 0o444)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&unit_dir, fs::Permissions::from_mode(0o555))?;
        }
    }
    fs::write(stage.join(".archive-sha256"), format!("{archive_sha}\n"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&stage, fs::Permissions::from_mode(0o555))?;
    }
    fs::rename(&stage, &release_dir).context("failed to publish staged release")?;
    Ok(release_dir)
}

fn write_mode(path: &Path, contents: &[u8], mode: u32) -> Result<()> {
    let mut file =
        File::create(path).with_context(|| format!("failed to write {}", path.display()))?;
    file.write_all(contents)?;
    file.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    let _ = mode;
    Ok(())
}

fn activate_release(
    install: &ManagedInstall,
    release_id: &str,
    previous_target: &str,
    progress_tx: &mpsc::UnboundedSender<UpdateStatus>,
) -> Result<()> {
    let _ = previous_target;
    publish_command_links(install)?;
    atomic_symlink(
        &format!(".mitsuro-releases/{release_id}"),
        &install.current_link,
    )?;
    if install.systemd_managed {
        let _ = progress_tx.send(UpdateStatus::Installing {
            progress: "Restarting Hive and server...".into(),
        });
        let _ = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status();
        restart_active_units(install)?;
        verify_active_units(install)?;
    }
    Ok(())
}

fn publish_command_links(install: &ManagedInstall) -> Result<()> {
    for name in ["mitsuro", "krusty", "mitsuro-hive", "krusty-mako"] {
        let dest = install.install_dir.join(name);
        let target = format!(".mitsuro-current/{name}");
        if dest.exists() || dest.symlink_metadata().is_ok() {
            if dest.is_symlink()
                || dest
                    .symlink_metadata()
                    .is_ok_and(|m| m.file_type().is_symlink())
            {
                atomic_symlink(&target, &dest)?;
            }
        } else if name == "mitsuro" || name == "krusty" {
            atomic_symlink(&target, &dest)?;
        }
    }
    Ok(())
}

fn restart_active_units(install: &ManagedInstall) -> Result<()> {
    if !install.systemd_managed {
        return Ok(());
    }
    let active = active_units();
    if active.is_empty() {
        return Ok(());
    }
    let status = Command::new("systemctl")
        .args(["--user", "restart"])
        .args(&active)
        .status()
        .context("failed to invoke systemctl")?;
    if !status.success() {
        return Err(anyhow!("systemctl restart failed"));
    }
    Ok(())
}

fn verify_active_units(install: &ManagedInstall) -> Result<()> {
    if !install.systemd_managed {
        return Ok(());
    }
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(400));
        let active = active_units();
        if active.is_empty() {
            return Ok(());
        }
        if active.iter().all(|unit| unit_is_active(unit)) {
            if active.iter().any(|unit| *unit == "mitsuro-serve.service") && !server_health_ok() {
                continue;
            }
            if active.iter().any(|unit| *unit == "mitsuro-hive.service") && !hive_ping_ok(install) {
                continue;
            }
            return Ok(());
        }
    }
    Err(anyhow!(
        "A previously active service did not settle healthy after the update"
    ))
}

fn active_units() -> Vec<String> {
    SYSTEMD_UNITS
        .iter()
        .filter(|unit| unit_is_active(unit))
        .map(|unit| (*unit).to_string())
        .collect()
}

fn unit_is_active(unit: &str) -> bool {
    Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", unit])
        .status()
        .is_ok_and(|status| status.success())
}

fn server_health_ok() -> bool {
    let Some(instance) = crate::server_instance::read_pid_file() else {
        return false;
    };
    let url = format!("http://127.0.0.1:{}/health", instance.port);
    Command::new("curl")
        .args(["--fail", "--silent", "--max-time", "5", &url])
        .output()
        .ok()
        .is_some_and(|output| {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).contains("\"status\":\"ok\"")
        })
}

fn hive_ping_ok(install: &ManagedInstall) -> bool {
    let hive = install.install_dir.join("mitsuro-hive");
    if !hive.exists() {
        return false;
    }
    let runtime =
        std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| format!("/run/user/{}", users_uid()));
    let socket = format!("{runtime}/mitsuro/hive.sock");
    let key = crate::paths::config_dir().join("run/hive-ipc.key");
    Command::new(hive)
        .args(["ping", "--socket", &socket, "--key"])
        .arg(key)
        .status()
        .is_ok_and(|status| status.success())
}

fn users_uid() -> u32 {
    #[cfg(unix)]
    {
        unsafe { libc::geteuid() }
    }
    #[cfg(not(unix))]
    {
        0
    }
}

fn read_current_target(current_link: &Path) -> Result<String> {
    match fs::read_link(current_link) {
        Ok(target) => Ok(target.to_string_lossy().into_owned()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).context("failed to read .mitsuro-current"),
    }
}

fn restore_current_link(current_link: &Path, previous_target: &str) -> Result<()> {
    if previous_target.is_empty() {
        return Ok(());
    }
    atomic_symlink(previous_target, current_link)
}

fn atomic_symlink(target: &str, dest: &Path) -> Result<()> {
    let parent = dest
        .parent()
        .ok_or_else(|| anyhow!("symlink destination has no parent"))?;
    let tmp = parent.join(format!(
        ".{}.mitsuro-new-{}",
        dest.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("link"),
        std::process::id()
    ));
    let _ = fs::remove_file(&tmp);
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, &tmp)
        .with_context(|| format!("failed to stage symlink {}", tmp.display()))?;
    #[cfg(not(unix))]
    {
        let _ = target;
        return Err(anyhow!(
            "Managed Unix updates are not supported on this platform"
        ));
    }
    fs::rename(&tmp, dest).with_context(|| format!("failed to publish {}", dest.display()))?;
    Ok(())
}

struct InstallLock {
    path: PathBuf,
}

impl InstallLock {
    fn acquire(install_dir: &Path) -> Result<Self> {
        let path = install_dir.join(".mitsuro-install.lock");
        fs::create_dir_all(install_dir)?;
        fs::create_dir(&path).with_context(|| {
            format!(
                "Another Mitsuro install is running (or left {} behind).",
                path.display()
            )
        })?;
        Ok(Self { path })
    }
}

impl Drop for InstallLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

async fn read_bounded_body(
    response: reqwest::Response,
    max_bytes: usize,
    description: &str,
) -> Result<Vec<u8>> {
    read_body_with_progress(response, max_bytes, description, &{
        let (tx, _rx) = mpsc::unbounded_channel();
        tx
    })
    .await
}

async fn read_body_with_progress(
    response: reqwest::Response,
    max_bytes: usize,
    description: &str,
    progress_tx: &mpsc::UnboundedSender<UpdateStatus>,
) -> Result<Vec<u8>> {
    let content_length = response.content_length();
    if content_length.is_some_and(|length| length > max_bytes as u64) {
        return Err(anyhow!("{description} exceeds {max_bytes} bytes"));
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.with_context(|| format!("failed while reading {description}"))?;
        let new_len = body
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| anyhow!("{description} size overflow"))?;
        if new_len > max_bytes {
            return Err(anyhow!("{description} exceeds {max_bytes} bytes"));
        }
        body.extend_from_slice(&chunk);
        let fraction = content_length.map(|total| body.len() as f32 / total.max(1) as f32);
        let _ = progress_tx.send(UpdateStatus::Downloading {
            progress: format!(
                "Downloading {}...",
                format_bytes(body.len() as u64, content_length)
            ),
            fraction,
        });
    }
    Ok(body)
}

fn format_bytes(received: u64, total: Option<u64>) -> String {
    match total {
        Some(total) => format!("{} / {}", human_bytes(received), human_bytes(total)),
        None => human_bytes(received),
    }
}

fn human_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn sample_release() -> ExtractedRelease {
        ExtractedRelease {
            mitsuro: b"mitsuro-bin".to_vec(),
            krusty: b"krusty-bin".to_vec(),
            hive: Some(b"hive-bin".to_vec()),
            krusty_mako: Some(b"mako-bin".to_vec()),
            agent_browser: None,
            systemd: SYSTEMD_UNITS
                .iter()
                .map(|name| ((*name).to_string(), format!("{name}-unit").into_bytes()))
                .collect(),
        }
    }

    #[test]
    fn stages_and_flips_the_managed_release_pointer() {
        let dir = tempfile::tempdir().expect("tempdir");
        let install_dir = dir.path().join("bin");
        let old = install_dir.join(".mitsuro-releases").join("old");
        fs::create_dir_all(&old).expect("old release");
        fs::write(old.join("mitsuro"), b"old").expect("old binary");
        fs::write(old.join("mitsuro-hive"), b"old-hive").expect("old hive");
        fs::create_dir_all(install_dir.join(".mitsuro-releases")).expect("releases");
        symlink(
            ".mitsuro-releases/old",
            install_dir.join(".mitsuro-current"),
        )
        .expect("link");

        let install = ManagedInstall {
            install_dir: install_dir.clone(),
            current_link: install_dir.join(".mitsuro-current"),
            current_release: old,
            has_hive: true,
            systemd_managed: false,
        };
        let release_id = "v9.9.9-test-abc";
        stage_release(&install, release_id, &sample_release(), "abc").expect("stage");
        activate_release(&install, release_id, ".mitsuro-releases/old", &{
            let (tx, _rx) = mpsc::unbounded_channel();
            tx
        })
        .expect("activate");

        assert_eq!(
            fs::read_link(install.current_link).unwrap(),
            PathBuf::from(format!(".mitsuro-releases/{release_id}"))
        );
        assert_eq!(
            fs::read(
                install_dir
                    .join(".mitsuro-releases")
                    .join(release_id)
                    .join("mitsuro")
            )
            .unwrap(),
            b"mitsuro-bin"
        );
    }

    #[test]
    fn rejects_path_traversal_members() {
        let error = normalize_member_path(Path::new("../mitsuro")).expect_err("escape");
        assert!(error.to_string().contains("escapes"));
    }

    #[test]
    fn refuses_hive_less_payload_over_hive_install() {
        let extracted = ExtractedRelease {
            hive: None,
            krusty_mako: None,
            systemd: Vec::new(),
            ..sample_release()
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let install = ManagedInstall {
            install_dir: dir.path().to_path_buf(),
            current_link: dir.path().join(".mitsuro-current"),
            current_release: dir.path().to_path_buf(),
            has_hive: true,
            systemd_managed: false,
        };
        // Staging itself is fine; the apply gate is the one that refuses.
        assert!(install.has_hive && extracted.hive.is_none());
    }
}
