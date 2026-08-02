use anyhow::{bail, Context, Result};
use std::{
    path::{Path, PathBuf},
    process::Output,
};
use tokio::fs;
use tracing::{debug, info, warn};

use super::{BunInstance, BunRuntime, BunRuntimeInner, BUN_BINARY, BUN_VERSION};

impl BunRuntime {
    pub fn new(http_client: reqwest::Client, data_dir: PathBuf) -> Self {
        Self {
            inner: std::sync::Arc::new(tokio::sync::RwLock::new(BunRuntimeInner {
                instance: None,
            })),
            http_client,
            data_dir,
        }
    }

    pub(crate) async fn get_instance(&self) -> Result<BunInstance> {
        {
            let inner = self.inner.read().await;
            if let Some(ref instance) = inner.instance {
                return Ok(instance.clone());
            }
        }

        match self.detect_system_bun().await {
            Ok(instance) => {
                info!("Using system Bun: {:?}", instance);
                let mut inner = self.inner.write().await;
                inner.instance = Some(instance.clone());
                return Ok(instance);
            }
            Err(e) => {
                debug!("System Bun not available: {}", e);
            }
        }

        let instance = self.install_managed_bun().await?;
        info!("Using managed Bun: {:?}", instance);
        let mut inner = self.inner.write().await;
        inner.instance = Some(instance.clone());
        Ok(instance)
    }

    async fn detect_system_bun(&self) -> Result<BunInstance> {
        let bun = which::which("bun").context("bun not found in PATH")?;

        let output = tokio::process::Command::new(&bun)
            .arg("--version")
            .output()
            .await
            .context("failed to run bun --version")?;

        if !output.status.success() {
            bail!("bun --version failed");
        }

        let version_str = String::from_utf8_lossy(&output.stdout);
        info!("Found system Bun version: {}", version_str.trim());

        Ok(BunInstance::System { bun })
    }

    async fn install_managed_bun(&self) -> Result<BunInstance> {
        let os = match std::env::consts::OS {
            "macos" => "darwin",
            "linux" => "linux",
            "windows" => "windows",
            other => bail!("Unsupported OS: {}", other),
        };

        let arch = match std::env::consts::ARCH {
            "x86_64" => "x64",
            "aarch64" => "aarch64",
            other => bail!("Unsupported architecture: {}", other),
        };

        let bun_dir = self.data_dir.join("bun");
        let bun_binary = bun_dir.join(BUN_BINARY);

        if fs::metadata(&bun_binary).await.is_ok() {
            let output: std::result::Result<Output, std::io::Error> =
                tokio::process::Command::new(&bun_binary)
                    .arg("--version")
                    .output()
                    .await;

            if let Ok(output) = output {
                if output.status.success() {
                    return Ok(BunInstance::Managed {
                        installation_path: bun_dir,
                    });
                }
            }
            warn!("Existing Bun installation invalid, reinstalling");
        }

        info!("Downloading Bun {}...", BUN_VERSION);

        let archive_name = if std::env::consts::OS == "windows" {
            format!("bun-windows-{}.zip", arch)
        } else {
            format!("bun-{}-{}.zip", os, arch)
        };

        let url = format!(
            "https://github.com/oven-sh/bun/releases/download/bun-v{}/{}",
            BUN_VERSION, archive_name
        );

        let response = self
            .http_client
            .get(&url)
            .send()
            .await
            .context("failed to download Bun")?;

        if !response.status().is_success() {
            bail!("Failed to download Bun: {}", response.status());
        }

        let bytes = response.bytes().await.context("failed to read response")?;
        fs::create_dir_all(&bun_dir).await?;
        self.extract_zip(&bytes, &bun_dir).await?;

        let extracted_dir = if std::env::consts::OS == "windows" {
            bun_dir.join(format!("bun-windows-{}", arch))
        } else {
            bun_dir.join(format!("bun-{}-{}", os, arch))
        };

        if extracted_dir.exists() {
            let extracted_binary = extracted_dir.join(BUN_BINARY);
            if extracted_binary.exists() {
                fs::rename(&extracted_binary, &bun_binary).await?;
            }
            let _ = fs::remove_dir_all(&extracted_dir).await;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&bun_binary).await?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&bun_binary, perms).await?;
        }

        info!("Bun installed to {}", bun_dir.display());

        Ok(BunInstance::Managed {
            installation_path: bun_dir,
        })
    }

    async fn extract_zip(&self, bytes: &[u8], dest: &Path) -> Result<()> {
        use std::io::Cursor;

        let reader = Cursor::new(bytes);
        let mut archive = zip::ZipArchive::new(reader).context("failed to open Bun zip archive")?;
        archive.extract(dest).context("failed to extract Bun zip")?;
        Ok(())
    }

    /// Get path to bun binary
    pub async fn binary_path(&self) -> Result<PathBuf> {
        match self.get_instance().await? {
            BunInstance::System { bun } => Ok(bun),
            BunInstance::Managed { installation_path } => Ok(installation_path.join(BUN_BINARY)),
        }
    }
}
