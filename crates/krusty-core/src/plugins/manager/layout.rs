use std::path::Path;

use anyhow::{bail, Context, Result};
use tokio::fs;

use super::storage::{load_lockfile, load_permissions, load_sources, load_trust_policy};
use super::PluginManager;

impl PluginManager {
    /// Ensure required plugin directories and config files exist.
    pub async fn ensure_layout(&self) -> Result<()> {
        ensure_manager_root(self).await?;
        ensure_real_directory(&self.installed_root(), "plugin install root").await?;
        ensure_transaction_roots(self).await?;
        ensure_real_directory(&self.active_root(), "active plugin root").await?;
        ensure_real_directory(&self.state_root(), "plugin state root").await?;
        ensure_real_directory(&self.index_root(), "plugin index root").await?;
        ensure_real_directory(&self.trust_root(), "plugin trust root").await?;

        load_lockfile(self).await?;
        load_trust_policy(self).await?;
        load_permissions(self).await?;
        load_sources(self).await?;

        // Fail closed if the manager root was replaced while its immediate
        // children were being initialized.
        ensure_real_directory(self.root(), "plugin manager root").await?;

        Ok(())
    }
}

/// Create the manager root without allowing an existing final component to be
/// followed when it is a symlink. Parent directories are outside the plugin
/// manager's ownership boundary and may already exist.
pub(super) async fn ensure_manager_root(manager: &PluginManager) -> Result<()> {
    if let Some(parent) = manager.root().parent() {
        fs::create_dir_all(parent).await.with_context(|| {
            format!(
                "failed to create plugin manager parent {}",
                parent.display()
            )
        })?;
    }
    ensure_real_directory(manager.root(), "plugin manager root").await
}

/// Validate the two directories that are security boundaries for immutable
/// plugin snapshots. This is repeated immediately before every transaction so
/// a path replaced after startup cannot redirect enumeration or publication.
pub(super) async fn ensure_transaction_roots(manager: &PluginManager) -> Result<()> {
    ensure_real_directory(&manager.installed_root(), "plugin install root").await?;
    ensure_real_directory(&manager.staging_root(), "plugin staging root").await?;
    ensure_real_directory(&manager.managed_root(), "managed plugin root").await
}

pub(super) async fn ensure_real_directory(path: &Path, description: &str) -> Result<()> {
    match fs::symlink_metadata(path).await {
        Ok(metadata) => validate_real_directory(path, description, &metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path)
                .await
                .with_context(|| format!("failed to create {description} {}", path.display()))?;
            let metadata = fs::symlink_metadata(path).await.with_context(|| {
                format!(
                    "failed to inspect newly created {description} {}",
                    path.display()
                )
            })?;
            validate_real_directory(path, description, &metadata)
        }
        Err(error) => Err(error)
            .with_context(|| format!("failed to inspect {description} {}", path.display())),
    }
}

fn validate_real_directory(
    path: &Path,
    description: &str,
    metadata: &std::fs::Metadata,
) -> Result<()> {
    let file_type = metadata.file_type();
    if file_type.is_symlink() || !file_type.is_dir() {
        bail!(
            "{description} must be a real directory, not a symlink or other filesystem entry: {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    use tempfile::tempdir;

    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn ensure_layout_rejects_staging_symlink_without_touching_target() {
        let temp = tempdir().expect("temporary directory");
        let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
        fs::create_dir_all(manager.installed_root())
            .await
            .expect("create install root");

        let victim = temp.path().join("external-victim");
        fs::create_dir_all(&victim)
            .await
            .expect("create external victim");
        fs::write(victim.join("keep.txt"), b"keep")
            .await
            .expect("write victim marker");
        symlink(&victim, manager.staging_root()).expect("create hostile staging symlink");

        let error = manager
            .ensure_layout()
            .await
            .expect_err("a staging symlink must fail closed");

        assert!(error
            .to_string()
            .contains("plugin staging root must be a real directory"));
        assert_eq!(
            fs::read(victim.join("keep.txt"))
                .await
                .expect("victim marker remains"),
            b"keep"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ensure_layout_rejects_symlink_manager_root_without_touching_target() {
        let temp = tempdir().expect("temporary directory");
        let victim = temp.path().join("external-victim");
        fs::create_dir_all(&victim)
            .await
            .expect("create external victim");
        fs::write(victim.join("keep.txt"), b"keep")
            .await
            .expect("write victim marker");
        let root = temp.path().join("plugins");
        symlink(&victim, &root).expect("create hostile manager-root symlink");
        let manager = PluginManager::new(reqwest::Client::new(), root);

        let error = manager
            .ensure_layout()
            .await
            .expect_err("a manager-root symlink must fail closed");

        assert!(error
            .to_string()
            .contains("plugin manager root must be a real directory"));
        assert_eq!(
            fs::read(victim.join("keep.txt"))
                .await
                .expect("victim marker remains"),
            b"keep"
        );
        assert!(!victim.join("installed").exists());
    }

    #[tokio::test]
    async fn ensure_layout_rejects_non_directory_manager_root() {
        let temp = tempdir().expect("temporary directory");
        let root = temp.path().join("plugins");
        fs::write(&root, b"not a directory")
            .await
            .expect("write hostile manager-root file");
        let manager = PluginManager::new(reqwest::Client::new(), root);

        let error = manager
            .ensure_layout()
            .await
            .expect_err("a non-directory manager root must fail closed");
        assert!(error
            .to_string()
            .contains("plugin manager root must be a real directory"));
    }
}
