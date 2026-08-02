use std::fs::{File, OpenOptions};
use std::io::ErrorKind;
use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use fs2::FileExt;
use tokio::sync::OwnedMutexGuard;

use super::PluginManager;

const LOCK_WAIT_TIMEOUT: Duration = Duration::from_secs(30);
const LOCK_WAIT_INTERVAL: Duration = Duration::from_millis(50);

/// Holds both the in-process mutex and the operating system's advisory lock.
///
/// The lock file is deliberately never unlinked. Reusing one stable inode
/// avoids the stale-lease ABA race where one process can delete a successor's
/// lock file after deciding that an earlier path-based lease is stale. The OS
/// releases this lock when the file descriptor closes, including after a crash.
#[derive(Debug)]
pub(super) struct PluginMutationGuard {
    _process_guard: OwnedMutexGuard<()>,
    lock_file: File,
}

impl Drop for PluginMutationGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.lock_file);
    }
}

impl PluginManager {
    pub(super) async fn acquire_mutation(&self) -> Result<PluginMutationGuard> {
        let acquisition = async {
            let process_guard = self.mutation_lock.clone().lock_owned().await;
            super::layout::ensure_manager_root(self).await?;
            let root_identity = manager_root_identity(self.root())?;
            let lock_path = self.root().join(".mutation.lock");
            let lock_file = open_lock_file(&lock_path)?;
            ensure_manager_root_identity(self.root(), root_identity)?;

            loop {
                match FileExt::try_lock_exclusive(&lock_file) {
                    Ok(()) => {
                        ensure_manager_root_identity(self.root(), root_identity)?;
                        ensure_open_lock_is_current(&lock_path, &lock_file)?;
                        return Ok(PluginMutationGuard {
                            _process_guard: process_guard,
                            lock_file,
                        });
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        tokio::time::sleep(LOCK_WAIT_INTERVAL).await;
                    }
                    Err(error) => {
                        return Err(error).with_context(|| {
                            format!(
                                "failed to acquire plugin mutation lock {}",
                                lock_path.display()
                            )
                        });
                    }
                }
            }
        };

        match tokio::time::timeout(LOCK_WAIT_TIMEOUT, acquisition).await {
            Ok(result) => result,
            Err(_) => {
                bail!("timed out waiting for another Mitsuro process to finish modifying plugins")
            }
        }
    }
}

fn open_lock_file(path: &Path) -> Result<File> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => validate_lock_path(path, &metadata)?,
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| {
                format!("failed to inspect plugin mutation lock {}", path.display())
            })
        }
    }

    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        options
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    let file = options
        .open(path)
        .with_context(|| format!("failed to open plugin mutation lock {}", path.display()))?;
    let opened_metadata = file.metadata().with_context(|| {
        format!(
            "failed to inspect opened plugin mutation lock {}",
            path.display()
        )
    })?;
    validate_opened_lock_file(path, &opened_metadata)?;
    let path_metadata = std::fs::symlink_metadata(path).with_context(|| {
        format!(
            "failed to re-inspect plugin mutation lock {}",
            path.display()
        )
    })?;
    validate_lock_path(path, &path_metadata)?;
    ensure_lock_path_identity(path, &opened_metadata, &path_metadata)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = file.metadata()?.permissions();
        permissions.set_mode(0o600);
        file.set_permissions(permissions).with_context(|| {
            format!(
                "failed to restrict plugin mutation lock permissions {}",
                path.display()
            )
        })?;
    }

    Ok(file)
}

fn ensure_open_lock_is_current(path: &Path, file: &File) -> Result<()> {
    let opened_metadata = file.metadata().with_context(|| {
        format!(
            "failed to inspect acquired plugin mutation lock {}",
            path.display()
        )
    })?;
    validate_opened_lock_file(path, &opened_metadata)?;
    let path_metadata = std::fs::symlink_metadata(path).with_context(|| {
        format!(
            "failed to inspect acquired plugin mutation lock path {}",
            path.display()
        )
    })?;
    validate_lock_path(path, &path_metadata)?;
    ensure_lock_path_identity(path, &opened_metadata, &path_metadata)
}

fn validate_lock_path(path: &Path, metadata: &std::fs::Metadata) -> Result<()> {
    let file_type = metadata.file_type();
    if file_type.is_symlink() || !file_type.is_file() {
        bail!(
            "plugin mutation lock must be a regular file, not a symlink or other filesystem entry: {}",
            path.display()
        );
    }
    Ok(())
}

fn validate_opened_lock_file(path: &Path, metadata: &std::fs::Metadata) -> Result<()> {
    if !metadata.file_type().is_file() {
        bail!(
            "opened plugin mutation lock is not a regular file: {}",
            path.display()
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        if metadata.nlink() != 1 {
            bail!(
                "plugin mutation lock must not have hard-link aliases: {}",
                path.display()
            );
        }
    }
    Ok(())
}

#[cfg(unix)]
fn ensure_lock_path_identity(
    path: &Path,
    opened: &std::fs::Metadata,
    current: &std::fs::Metadata,
) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    if opened.dev() != current.dev() || opened.ino() != current.ino() {
        bail!(
            "plugin mutation lock changed identity while being opened: {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_lock_path_identity(
    _path: &Path,
    _opened: &std::fs::Metadata,
    _current: &std::fs::Metadata,
) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
#[derive(Clone, Copy)]
struct ManagerRootIdentity {
    device: u64,
    inode: u64,
}

#[cfg(not(unix))]
#[derive(Clone, Copy)]
struct ManagerRootIdentity;

fn manager_root_identity(path: &Path) -> Result<ManagerRootIdentity> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect plugin manager root {}", path.display()))?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() || !file_type.is_dir() {
        bail!(
            "plugin manager root must be a real directory, not a symlink or other filesystem entry: {}",
            path.display()
        );
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        Ok(ManagerRootIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    #[cfg(not(unix))]
    {
        Ok(ManagerRootIdentity)
    }
}

fn ensure_manager_root_identity(path: &Path, expected: ManagerRootIdentity) -> Result<()> {
    let current = manager_root_identity(path)?;
    #[cfg(unix)]
    if current.device != expected.device || current.inode != expected.inode {
        bail!(
            "plugin manager root changed identity while opening its mutation lock: {}",
            path.display()
        );
    }
    #[cfg(not(unix))]
    let _ = (current, expected);
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::symlink;

    use tempfile::tempdir;

    use super::*;

    #[tokio::test]
    async fn mutation_lock_rejects_symlink_without_touching_target() {
        let temp = tempdir().expect("temporary directory");
        let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
        manager.ensure_layout().await.expect("ensure layout");
        let victim = temp.path().join("victim.txt");
        std::fs::write(&victim, b"keep").expect("write victim");
        symlink(&victim, manager.root().join(".mutation.lock"))
            .expect("create hostile lock symlink");

        let error = manager
            .acquire_mutation()
            .await
            .expect_err("a lock symlink must fail closed");
        assert!(error
            .to_string()
            .contains("plugin mutation lock must be a regular file"));
        assert_eq!(std::fs::read(&victim).expect("read victim"), b"keep");
    }
}
