use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

const MAX_TRUST_STORE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectAgentExtensionTrustStatus {
    pub project_path: PathBuf,
    pub trusted: bool,
    pub trust_store_path: PathBuf,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct TrustStoreData {
    version: u32,
    trusted_projects: BTreeSet<String>,
}

pub(super) struct ProjectAgentExtensionTrustStore {
    path: PathBuf,
}

impl ProjectAgentExtensionTrustStore {
    pub(super) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub(super) fn status(&self, project_dir: &Path) -> Result<ProjectAgentExtensionTrustStatus> {
        let project_path = canonical_project_path(project_dir)?;
        let data = self.read()?;
        Ok(ProjectAgentExtensionTrustStatus {
            trusted: data
                .trusted_projects
                .contains(&project_path.to_string_lossy().into_owned()),
            project_path,
            trust_store_path: self.path.clone(),
        })
    }

    pub(super) fn set_trusted(
        &self,
        project_dir: &Path,
        trusted: bool,
    ) -> Result<ProjectAgentExtensionTrustStatus> {
        let project_path = canonical_project_path(project_dir)?;
        let parent = self
            .path
            .parent()
            .context("project-extension trust store has no parent directory")?;
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
        restrict_dir_permissions(parent)?;

        let lock_path = self.path.with_extension("lock");
        let lock = open_private_file(&lock_path)?;
        lock.lock_exclusive()
            .with_context(|| format!("failed to lock {}", lock_path.display()))?;

        let mut data = self.read()?;
        data.version = 1;
        let key = project_path.to_string_lossy().into_owned();
        if trusted {
            data.trusted_projects.insert(key);
        } else {
            data.trusted_projects.remove(&key);
        }
        self.write_atomic(&data)?;
        FileExt::unlock(&lock).ok();

        Ok(ProjectAgentExtensionTrustStatus {
            project_path,
            trusted,
            trust_store_path: self.path.clone(),
        })
    }

    fn read(&self) -> Result<TrustStoreData> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(TrustStoreData::default())
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to open {}", self.path.display()))
            }
        };
        let metadata = file.metadata()?;
        if metadata.len() > MAX_TRUST_STORE_BYTES {
            bail!(
                "project-extension trust store exceeds {} bytes",
                MAX_TRUST_STORE_BYTES
            );
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(MAX_TRUST_STORE_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_TRUST_STORE_BYTES {
            bail!(
                "project-extension trust store exceeds {} bytes",
                MAX_TRUST_STORE_BYTES
            );
        }
        serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse {}", self.path.display()))
    }

    fn write_atomic(&self, data: &TrustStoreData) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(data)?;
        if bytes.len() as u64 > MAX_TRUST_STORE_BYTES {
            bail!(
                "project-extension trust store exceeds {} bytes",
                MAX_TRUST_STORE_BYTES
            );
        }
        let parent = self.path.parent().context("trust store has no parent")?;
        let temp_path = parent.join(format!(
            ".project-extension-trust.{}.{}.tmp",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let mut temp = open_private_file(&temp_path)?;
        temp.set_len(0)?;
        temp.seek(SeekFrom::Start(0))?;
        temp.write_all(&bytes)?;
        temp.sync_all()?;
        fs::rename(&temp_path, &self.path).with_context(|| {
            format!(
                "failed to replace project-extension trust store {}",
                self.path.display()
            )
        })?;
        restrict_file_permissions(&self.path)?;
        if let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    }
}

fn canonical_project_path(project_dir: &Path) -> Result<PathBuf> {
    project_dir.canonicalize().with_context(|| {
        format!(
            "failed to canonicalize project path {}",
            project_dir.display()
        )
    })
}

fn open_private_file(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    restrict_file_permissions(path)?;
    Ok(file)
}

#[cfg(unix)]
fn restrict_dir_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_dir_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn restrict_file_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_file_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_trust_is_fail_closed_and_user_owned() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        let store = ProjectAgentExtensionTrustStore::new(temp.path().join("user/trust.json"));

        assert!(!store.status(&project).unwrap().trusted);
        assert!(store.set_trusted(&project, true).unwrap().trusted);
        assert!(store.status(&project).unwrap().trusted);
        assert!(!store.set_trusted(&project, false).unwrap().trusted);
        assert!(!store.status(&project).unwrap().trusted);
    }
}
