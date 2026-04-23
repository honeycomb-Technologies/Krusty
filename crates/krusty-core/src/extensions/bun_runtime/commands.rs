use anyhow::{bail, Context, Result};
use std::{path::Path, process::Output};

use super::{
    path_with_bun_prepended, BunInstance, BunPackageInfo, BunRuntime, PackageJson, BUN_BINARY,
};

impl BunRuntime {
    /// Run a bun subcommand (replaces npm)
    pub async fn run_bun_subcommand(
        &self,
        directory: Option<&Path>,
        subcommand: &str,
        args: &[&str],
    ) -> Result<Output> {
        let instance = self.get_instance().await?;

        let bun_binary = match &instance {
            BunInstance::System { bun } => bun.clone(),
            BunInstance::Managed { installation_path } => installation_path.join(BUN_BINARY),
        };

        let env_path = path_with_bun_prepended(&bun_binary);

        let mut command = tokio::process::Command::new(&bun_binary);
        if let Some(path) = env_path {
            command.env("PATH", path);
        }
        command.arg(subcommand);
        command.args(args);

        if let Some(dir) = directory {
            command.current_dir(dir);
        }

        let output = command.output().await.context("failed to run bun")?;

        if !output.status.success() {
            bail!(
                "bun {} failed:
stdout: {}
stderr: {}",
                subcommand,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(output)
    }

    /// Get latest version of an npm package (bun compatible)
    pub async fn npm_package_latest_version(&self, package: &str) -> Result<String> {
        let output = self
            .run_bun_subcommand(None, "pm", &["info", package, "--json"])
            .await?;

        let info: BunPackageInfo = serde_json::from_slice(&output.stdout)
            .context("failed to parse bun pm info response")?;

        info.version
            .or(info.dist_tags.and_then(|dt| dt.latest))
            .with_context(|| format!("no version found for package {}", package))
    }

    /// Get installed version of a package
    pub async fn npm_package_installed_version(
        &self,
        directory: &Path,
        package: &str,
    ) -> Result<Option<String>> {
        let package_json = directory
            .join("node_modules")
            .join(package)
            .join("package.json");

        match tokio::fs::read_to_string(&package_json).await {
            Ok(content) => {
                let pkg: PackageJson = serde_json::from_str(&content)?;
                Ok(Some(pkg.version))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Install packages (uses bun install)
    pub async fn npm_install_packages(
        &self,
        directory: &Path,
        packages: &[(&str, &str)],
    ) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        let package_specs: Vec<String> = packages
            .iter()
            .map(|(name, version)| format!("{}@{}", name, version))
            .collect();

        let args: Vec<&str> = package_specs.iter().map(|s| s.as_str()).collect();

        self.run_bun_subcommand(Some(directory), "add", &args)
            .await?;
        Ok(())
    }
}
