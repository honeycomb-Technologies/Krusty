use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use tokio::fs;
use tracing::warn;

use super::layout::{ensure_real_directory, ensure_transaction_roots};
use super::storage::{
    load_lockfile, load_permissions, read_installed_from_lock_entry, save_lockfile,
    save_permissions, sync_directory,
};
use super::PluginManager;
use crate::plugins::{InstalledPlugin, PluginLockEntry};

pub(super) async fn create_staging_root(manager: &PluginManager) -> Result<PathBuf> {
    ensure_transaction_roots(manager).await?;
    let root = manager
        .staging_root()
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir(&root)
        .await
        .with_context(|| format!("failed to create plugin staging root {}", root.display()))?;
    ensure_real_directory(&root, "plugin transaction root").await?;
    // Return the manager-constructed lexical path. Canonicalizing here would
    // turn a hostile root symlink into the authority used by later checks.
    Ok(root)
}

/// Atomically publishes a complete staged snapshot, then atomically swaps all
/// associated lock entries. A crash before the lock swap leaves an orphan that
/// `reconcile_plugins` can safely remove; a lock write failure rolls back the snapshot.
pub(super) async fn commit_staged_install(
    manager: &PluginManager,
    staging_root: &Path,
    mut staged_entries: Vec<PluginLockEntry>,
) -> Result<Vec<InstalledPlugin>> {
    if staged_entries.is_empty() {
        bail!("cannot commit an empty plugin install transaction");
    }
    ensure_transaction_roots(manager).await?;
    if staging_root.parent() != Some(manager.staging_root().as_path()) {
        bail!(
            "plugin transaction root is not a direct child of the staging directory: {}",
            staging_root.display()
        );
    }
    let staging_identity = real_directory_identity(staging_root, "plugin transaction root").await?;

    let mut ids = BTreeSet::new();
    for entry in &staged_entries {
        if !ids.insert(entry.id.clone()) {
            bail!("package contains duplicate plugin id '{}'", entry.id);
        }
        let package_path = entry
            .package_path
            .as_ref()
            .context("staged plugin lock entry is missing package_path")?;
        if !package_path.starts_with(staging_root) {
            bail!(
                "staged package path escapes transaction root: {}",
                package_path.display()
            );
        }
        // Validate identity, version, manifest, component existence, and path
        // containment before making the snapshot visible in the lockfile.
        read_installed_from_lock_entry(manager, entry).await?;
    }

    let transaction_id = staging_root
        .file_name()
        .context("plugin staging root has no transaction id")?;
    let published_root = manager.managed_root().join(transaction_id);
    let staged_relatives = staged_entries
        .iter()
        .map(|entry| {
            entry
                .package_path
                .as_ref()
                .context("staged plugin lock entry is missing package_path")?
                .strip_prefix(staging_root)
                .map(Path::to_path_buf)
                .context("staged package path escapes transaction root")
        })
        .collect::<Result<Vec<_>>>()?;
    ensure_transaction_roots(manager).await?;
    ensure_directory_identity(staging_root, "plugin transaction root", &staging_identity).await?;
    fs::rename(staging_root, &published_root)
        .await
        .with_context(|| {
            format!(
                "failed to atomically publish plugin snapshot {}",
                published_root.display()
            )
        })?;
    if let Err(error) = ensure_transaction_roots(manager).await {
        rollback_published_root(manager, &published_root).await;
        return Err(error).context("plugin transaction roots changed during publication");
    }
    if let Err(error) = ensure_directory_identity(
        &published_root,
        "published plugin snapshot",
        &staging_identity,
    )
    .await
    {
        rollback_published_root(manager, &published_root).await;
        return Err(error).context("published plugin snapshot failed identity validation");
    }
    if let Err(error) = sync_directory(&manager.managed_root()) {
        rollback_published_root(manager, &published_root).await;
        return Err(error).context("failed to sync managed plugin root after publication");
    }

    for (entry, relative) in staged_entries.iter_mut().zip(staged_relatives) {
        entry.package_path = Some(published_root.join(relative));
        entry.managed_root = Some(published_root.clone());
    }

    // Construct every public descriptor from the paths that will enter the
    // lockfile. Once the lock swap succeeds there must be no fallible reread
    // that can report a failed install even though it is already committed.
    let mut installed = Vec::with_capacity(staged_entries.len());
    for entry in &staged_entries {
        match read_installed_from_lock_entry(manager, entry).await {
            Ok(plugin) => installed.push(plugin),
            Err(error) => {
                rollback_published_root(manager, &published_root).await;
                return Err(error).context(format!(
                    "published plugin '{}' failed validation before lockfile commit",
                    entry.id
                ));
            }
        }
    }

    let previous_lock = match load_lockfile(manager).await {
        Ok(lock) => lock,
        Err(error) => {
            rollback_published_root(manager, &published_root).await;
            return Err(error).context("failed to load plugin lockfile before commit");
        }
    };
    let mut next_lock = previous_lock.clone();
    let replaced_sources: BTreeSet<String> = staged_entries
        .iter()
        .filter_map(|entry| entry.source.clone())
        .collect();
    next_lock.plugins.retain(|entry| {
        !ids.contains(entry.id.as_str())
            && entry
                .source
                .as_ref()
                .map(|source| !replaced_sources.contains(source))
                .unwrap_or(true)
    });
    next_lock.plugins.extend(staged_entries.clone());
    next_lock.plugins.sort_by(|a, b| a.id.cmp(&b.id));

    if let Err(error) = save_lockfile(manager, &next_lock).await {
        rollback_published_root(manager, &published_root).await;
        return Err(error).context("failed to commit plugin lockfile transaction");
    }

    let current_ids: BTreeSet<&str> = next_lock
        .plugins
        .iter()
        .map(|entry| entry.id.as_str())
        .collect();
    if let Ok(mut permissions) = load_permissions(manager).await {
        let previous_len = permissions.plugins.len();
        permissions
            .plugins
            .retain(|plugin_id, _| current_ids.contains(plugin_id.as_str()));
        if permissions.plugins.len() != previous_len {
            if let Err(error) = save_permissions(manager, &permissions).await {
                warn!(
                    "plugin install committed but obsolete permission grants need reconciliation: {}",
                    error
                );
            }
        }
    }

    cleanup_replaced_roots(manager, &previous_lock.plugins, &next_lock.plugins).await;
    Ok(installed)
}

async fn rollback_published_root(manager: &PluginManager, published_root: &Path) {
    if let Err(error) = remove_manager_owned_root(manager, published_root).await {
        warn!(
            "failed to roll back uncommitted plugin snapshot {}: {}",
            published_root.display(),
            error
        );
    }
}

async fn cleanup_replaced_roots(
    manager: &PluginManager,
    previous: &[PluginLockEntry],
    current: &[PluginLockEntry],
) {
    let referenced: BTreeSet<PathBuf> = current
        .iter()
        .filter_map(|entry| entry.managed_root.clone())
        .collect();
    let old_roots: BTreeSet<PathBuf> = previous
        .iter()
        .filter_map(|entry| entry.managed_root.clone())
        .filter(|root| !referenced.contains(root))
        .collect();

    for root in old_roots {
        if let Err(error) = remove_manager_owned_root(manager, &root).await {
            warn!(
                "plugin update committed but old snapshot {} could not be removed: {}",
                root.display(),
                error
            );
        }
    }
}

pub(super) async fn remove_manager_owned_root(manager: &PluginManager, root: &Path) -> Result<()> {
    let parent = root
        .parent()
        .context("managed plugin root has no parent directory")?;

    // Authorize only manager-constructed lexical descendants. Never
    // canonicalize these roots: doing so would bless the target of a hostile
    // `.managed`/`.staging` symlink as manager-owned authority.
    let managed_root = manager.managed_root();
    let staging_root = manager.staging_root();
    let installed_root = manager.installed_root();
    let direct_transaction_child =
        parent == managed_root.as_path() || parent == staging_root.as_path();
    let legacy_version_child = parent.parent() == Some(installed_root.as_path());
    if !direct_transaction_child && !legacy_version_child {
        bail!(
            "refusing to remove path that is not a direct manager-owned snapshot: {}",
            root.display()
        );
    }

    if direct_transaction_child {
        ensure_real_directory(parent, "plugin snapshot parent").await?;
    } else {
        ensure_real_directory(&installed_root, "plugin install root").await?;
        ensure_real_directory(parent, "legacy plugin id root").await?;
    }

    // Only inspect the child after its parent has been proven to be the real
    // manager-owned directory. Otherwise even `symlink_metadata(root)` could
    // traverse a hostile parent symlink into an external tree.
    let metadata = match fs::symlink_metadata(root).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };

    // Delete the exact directory entry. Never canonicalize a symlink and then
    // recursively delete its target: a corrupted lockfile or orphan symlink
    // must not be able to erase a sibling snapshot.
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        fs::remove_file(root).await.with_context(|| {
            format!("failed to remove plugin snapshot entry {}", root.display())
        })?;
        return Ok(());
    }

    fs::remove_dir_all(root)
        .await
        .with_context(|| format!("failed to remove plugin snapshot {}", root.display()))
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DirectoryIdentity {
    device: u64,
    inode: u64,
}

#[cfg(not(unix))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DirectoryIdentity;

async fn real_directory_identity(path: &Path, description: &str) -> Result<DirectoryIdentity> {
    let metadata = fs::symlink_metadata(path)
        .await
        .with_context(|| format!("failed to inspect {description} {}", path.display()))?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() || !file_type.is_dir() {
        bail!(
            "{description} must be a real directory, not a symlink or other filesystem entry: {}",
            path.display()
        );
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        Ok(DirectoryIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    #[cfg(not(unix))]
    {
        Ok(DirectoryIdentity)
    }
}

async fn ensure_directory_identity(
    path: &Path,
    description: &str,
    expected: &DirectoryIdentity,
) -> Result<()> {
    let actual = real_directory_identity(path, description).await?;
    if actual != *expected {
        bail!(
            "{description} changed identity during plugin transaction: {}",
            path.display()
        );
    }
    Ok(())
}
