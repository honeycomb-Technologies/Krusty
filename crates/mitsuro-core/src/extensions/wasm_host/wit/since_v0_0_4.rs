//! WIT bindings for extension API v0.0.4

use super::since_v0_8_0 as latest;
use crate::extensions::types::WorktreeDelegate;
use crate::extensions::wasm_host::WasmState;
use anyhow::Result;
use semver::Version;
use std::sync::{Arc, OnceLock};
use wasmtime::component::{Linker, Resource};

pub const MIN_VERSION: Version = Version::new(0, 0, 4);

wasmtime::component::bindgen!({
    async: true,
    trappable_imports: true,
    path: "src/extensions/wit/since_v0.0.4",
    with: {
        "worktree": ExtensionWorktree,
        "zed:extension/github": latest::zed::extension::github,
        "zed:extension/platform": latest::zed::extension::platform,
    },
});

pub type ExtensionWorktree = Arc<dyn WorktreeDelegate>;

pub fn linker() -> &'static Linker<WasmState> {
    static LINKER: OnceLock<Linker<WasmState>> = OnceLock::new();
    LINKER.get_or_init(|| {
        super::new_linker(Extension::add_to_linker).expect("failed to create WASM linker")
    })
}

impl From<Command> for latest::Command {
    fn from(value: Command) -> Self {
        Self {
            command: value.command,
            args: value.args,
            env: value.env,
        }
    }
}

impl From<LanguageServerInstallationStatus> for latest::LanguageServerInstallationStatus {
    fn from(value: LanguageServerInstallationStatus) -> Self {
        match value {
            LanguageServerInstallationStatus::None => Self::None,
            LanguageServerInstallationStatus::Downloading => Self::Downloading,
            LanguageServerInstallationStatus::CheckingForUpdate => Self::CheckingForUpdate,
            LanguageServerInstallationStatus::Failed(msg) => Self::Failed(msg),
        }
    }
}

impl From<DownloadedFileType> for latest::DownloadedFileType {
    fn from(value: DownloadedFileType) -> Self {
        match value {
            DownloadedFileType::Gzip => Self::Gzip,
            DownloadedFileType::GzipTar => Self::GzipTar,
            DownloadedFileType::Zip => Self::Zip,
            DownloadedFileType::Uncompressed => Self::Uncompressed,
        }
    }
}

impl HostWorktree for WasmState {
    async fn read_text_file(
        &mut self,
        delegate: Resource<ExtensionWorktree>,
        path: String,
    ) -> wasmtime::Result<Result<String, String>> {
        latest::HostWorktree::read_text_file(self, delegate, path).await
    }

    async fn shell_env(
        &mut self,
        delegate: Resource<ExtensionWorktree>,
    ) -> wasmtime::Result<EnvVars> {
        latest::HostWorktree::shell_env(self, delegate).await
    }

    async fn which(
        &mut self,
        delegate: Resource<ExtensionWorktree>,
        binary_name: String,
    ) -> wasmtime::Result<Option<String>> {
        latest::HostWorktree::which(self, delegate, binary_name).await
    }

    async fn drop(&mut self, delegate: Resource<ExtensionWorktree>) -> wasmtime::Result<()> {
        latest::HostWorktree::drop(self, delegate).await
    }
}

impl ExtensionImports for WasmState {
    async fn current_platform(
        &mut self,
    ) -> wasmtime::Result<(
        zed::extension::platform::Os,
        zed::extension::platform::Architecture,
    )> {
        latest::zed::extension::platform::Host::current_platform(self).await
    }

    async fn node_binary_path(&mut self) -> wasmtime::Result<Result<String, String>> {
        latest::zed::extension::nodejs::Host::node_binary_path(self).await
    }

    async fn npm_package_latest_version(
        &mut self,
        package: String,
    ) -> wasmtime::Result<Result<String, String>> {
        latest::zed::extension::nodejs::Host::npm_package_latest_version(self, package).await
    }

    async fn npm_package_installed_version(
        &mut self,
        package: String,
    ) -> wasmtime::Result<Result<Option<String>, String>> {
        latest::zed::extension::nodejs::Host::npm_package_installed_version(self, package).await
    }

    async fn npm_install_package(
        &mut self,
        package: String,
        version: String,
    ) -> wasmtime::Result<Result<(), String>> {
        latest::zed::extension::nodejs::Host::npm_install_package(self, package, version).await
    }

    async fn latest_github_release(
        &mut self,
        repo: String,
        options: zed::extension::github::GithubReleaseOptions,
    ) -> wasmtime::Result<Result<zed::extension::github::GithubRelease, String>> {
        latest::zed::extension::github::Host::latest_github_release(self, repo, options).await
    }

    async fn download_file(
        &mut self,
        url: String,
        path: String,
        file_type: DownloadedFileType,
    ) -> wasmtime::Result<Result<(), String>> {
        latest::ExtensionImports::download_file(self, url, path, file_type.into()).await
    }

    async fn make_file_executable(&mut self, path: String) -> wasmtime::Result<Result<(), String>> {
        latest::ExtensionImports::make_file_executable(self, path).await
    }

    async fn set_language_server_installation_status(
        &mut self,
        server_name: String,
        status: LanguageServerInstallationStatus,
    ) -> wasmtime::Result<()> {
        latest::ExtensionImports::set_language_server_installation_status(
            self,
            server_name,
            status.into(),
        )
        .await
    }
}
