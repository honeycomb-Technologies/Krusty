//! How this Mitsuro binary was installed, and whether we can apply a release.

use std::path::{Path, PathBuf};

use super::checker::paths::detect_repo_path;
use super::checker::policy::MANAGED_UPDATE_MESSAGE;

const MANAGED_CURRENT: &str = ".mitsuro-current";
const MANAGED_RELEASES: &str = ".mitsuro-releases";
const MANAGED_SYSTEMD_MARKER: &str = ".mitsuro-systemd-managed";
const HIVE_BINARY: &str = "mitsuro-hive";

/// How a running binary can be upgraded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateApplyPolicy {
    /// Flip the shell-installer release pointer and restart managed services.
    ManagedRelease,
    /// Tell the user the exact external command.
    External { command: String },
    /// No in-product apply path.
    Unavailable { reason: String },
}

impl UpdateApplyPolicy {
    pub fn can_apply(&self) -> bool {
        matches!(self, Self::ManagedRelease)
    }

    pub fn notice_hint(&self) -> &'static str {
        match self {
            Self::ManagedRelease => "Ctrl+U to install",
            Self::External { .. } | Self::Unavailable { .. } => "see update guidance",
        }
    }

    pub fn guidance(&self) -> String {
        match self {
            Self::ManagedRelease => {
                "Press Ctrl+U or run `mitsuro update --apply` to install this release.".to_string()
            }
            Self::External { command } => format!("Update with: {command}"),
            Self::Unavailable { reason } => reason.clone(),
        }
    }
}

/// Detected install channel for the running executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateChannel {
    ManagedUnix(ManagedInstall),
    Homebrew,
    SourceBuild,
    WindowsDirect,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedInstall {
    pub install_dir: PathBuf,
    pub current_link: PathBuf,
    pub current_release: PathBuf,
    pub has_hive: bool,
    pub systemd_managed: bool,
}

impl UpdateChannel {
    pub fn detect() -> Self {
        let exe = std::env::current_exe().ok();
        let cwd = std::env::current_dir().ok();
        detect_channel(exe.as_deref(), cwd.as_deref())
    }

    pub fn apply_policy(&self) -> UpdateApplyPolicy {
        match self {
            Self::ManagedUnix(_) => UpdateApplyPolicy::ManagedRelease,
            Self::Homebrew => UpdateApplyPolicy::External {
                command: "brew upgrade BurgessTG/tap/mitsuro".to_string(),
            },
            Self::SourceBuild => UpdateApplyPolicy::Unavailable {
                reason: "This session is a source build. Pull the repository and rebuild, or install a release with install.sh.".to_string(),
            },
            Self::WindowsDirect => UpdateApplyPolicy::External {
                command: "rerun the checksum-verifying installer from https://github.com/honeycomb-Technologies/Mitsuro/blob/main/install.sh".to_string(),
            },
            Self::Unknown => UpdateApplyPolicy::Unavailable {
                reason: MANAGED_UPDATE_MESSAGE.to_string(),
            },
        }
    }

    pub fn managed_install(&self) -> Option<&ManagedInstall> {
        match self {
            Self::ManagedUnix(install) => Some(install),
            _ => None,
        }
    }
}

pub fn detect_channel(exe: Option<&Path>, cwd: Option<&Path>) -> UpdateChannel {
    if cfg!(windows) {
        return UpdateChannel::WindowsDirect;
    }

    if let Some(exe) = exe {
        if is_source_build_exe(exe) {
            return UpdateChannel::SourceBuild;
        }
        if is_homebrew_exe(exe) {
            return UpdateChannel::Homebrew;
        }
        if let Some(managed) = detect_managed_unix(exe) {
            return UpdateChannel::ManagedUnix(managed);
        }
    }

    // cwd alone must not classify a managed binary as a source build.
    let _ = cwd;
    if detect_repo_path().is_some() {
        if let Some(exe) = exe {
            if is_source_build_exe(exe) {
                return UpdateChannel::SourceBuild;
            }
        }
    }

    UpdateChannel::Unknown
}

fn is_source_build_exe(exe: &Path) -> bool {
    let mut components = exe.components().rev();
    let file = components.next();
    let parent = components.next();
    let target = components.next();
    let looks_like_binary =
        file.is_some_and(|name| name.as_os_str() == "mitsuro" || name.as_os_str() == "mitsuro.exe");
    let in_profile =
        parent.is_some_and(|name| matches!(name.as_os_str().to_str(), Some("debug" | "release")));
    let in_target = target.is_some_and(|name| name.as_os_str() == "target");
    looks_like_binary && in_profile && in_target
}

fn is_homebrew_exe(exe: &Path) -> bool {
    let text = exe.to_string_lossy();
    text.contains("/Cellar/mitsuro/")
        || text.contains("/linuxbrew/")
        || text.contains("/Homebrew/Cellar/")
}

fn detect_managed_unix(exe: &Path) -> Option<ManagedInstall> {
    let exe_parent = exe.parent()?;
    if exe_parent.file_name()?.to_str()? == MANAGED_CURRENT {
        return managed_from_install_dir(exe_parent.parent()?);
    }
    let releases = exe_parent.parent()?;
    if releases.file_name()?.to_str()? == MANAGED_RELEASES {
        return managed_from_install_dir(releases.parent()?);
    }
    None
}

fn managed_from_install_dir(install_dir: &Path) -> Option<ManagedInstall> {
    let current_link = install_dir.join(MANAGED_CURRENT);
    if current_link.symlink_metadata().is_err() {
        return None;
    }
    let current_release = std::fs::read_link(&current_link)
        .ok()
        .map(|target| {
            if target.is_absolute() {
                target
            } else {
                install_dir.join(target)
            }
        })
        .or_else(|| current_link.canonicalize().ok())?;
    let current_release = if current_release.is_dir() {
        current_release
    } else {
        current_release.parent()?.to_path_buf()
    };
    Some(ManagedInstall {
        has_hive: current_release.join(HIVE_BINARY).is_file(),
        systemd_managed: install_dir.join(MANAGED_SYSTEMD_MARKER).is_file(),
        install_dir: install_dir.to_path_buf(),
        current_link,
        current_release,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

    #[test]
    fn source_build_is_detected_from_target_profile_only() {
        let exe = Path::new("/home/dev/Mitsuro/target/release/mitsuro");
        assert!(is_source_build_exe(exe));
        assert_eq!(
            detect_channel(Some(exe), Some(Path::new("/home/dev/Mitsuro"))),
            UpdateChannel::SourceBuild
        );
    }

    #[test]
    fn managed_install_is_not_a_source_build_just_because_cwd_is_the_repo() {
        let dir = tempfile::tempdir().expect("tempdir");
        let install_dir = dir.path().join("bin");
        let release = install_dir
            .join(MANAGED_RELEASES)
            .join("v1.0.0-x86_64-unknown-linux-gnu-deadbeef");
        fs::create_dir_all(&release).expect("release dir");
        fs::write(release.join("mitsuro"), b"bin").expect("binary");
        fs::write(release.join(HIVE_BINARY), b"hive").expect("hive");
        symlink(
            Path::new(".mitsuro-releases/v1.0.0-x86_64-unknown-linux-gnu-deadbeef"),
            install_dir.join(MANAGED_CURRENT),
        )
        .expect("current link");
        let exe = release.join("mitsuro");
        match detect_channel(Some(&exe), Some(Path::new("/home/dev/Mitsuro"))) {
            UpdateChannel::ManagedUnix(managed) => {
                assert!(managed.has_hive);
                assert_eq!(managed.install_dir, install_dir);
            }
            other => panic!("expected managed unix, got {other:?}"),
        }
    }

    #[test]
    fn homebrew_cellar_is_external() {
        let exe = Path::new("/opt/homebrew/Cellar/mitsuro/0.9.22/bin/mitsuro");
        assert_eq!(detect_channel(Some(exe), None), UpdateChannel::Homebrew);
        assert!(!UpdateChannel::Homebrew.apply_policy().can_apply());
    }
}
