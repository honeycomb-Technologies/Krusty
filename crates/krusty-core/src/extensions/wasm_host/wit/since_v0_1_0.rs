//! WIT bindings for extension API v0.1.0

use super::since_v0_8_0 as latest;
use crate::extensions::types::WorktreeDelegate;
use crate::extensions::wasm_host::WasmState;
use anyhow::Result;
use semver::Version;
use std::sync::{Arc, OnceLock};
use wasmtime::component::{Linker, Resource};

pub const MIN_VERSION: Version = Version::new(0, 1, 0);

wasmtime::component::bindgen!({
    async: true,
    trappable_imports: true,
    path: "src/extensions/wit/since_v0.1.0",
    with: {
        "worktree": ExtensionWorktree,
        "zed:extension/github": latest::zed::extension::github,
        "zed:extension/platform": latest::zed::extension::platform,
        "zed:extension/nodejs": latest::zed::extension::nodejs,
        "zed:extension/http-client": latest::zed::extension::http_client,
        "zed:extension/common": latest::zed::extension::common,
        "zed:extension/slash-command": latest::zed::extension::slash_command,
    },
});

pub type ExtensionWorktree = Arc<dyn WorktreeDelegate>;

pub fn linker() -> &'static Linker<WasmState> {
    static LINKER: OnceLock<Linker<WasmState>> = OnceLock::new();
    LINKER.get_or_init(|| super::new_linker(Extension::add_to_linker))
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

impl From<SettingsLocation> for latest::SettingsLocation {
    fn from(value: SettingsLocation) -> Self {
        Self {
            worktree_id: value.worktree_id,
            path: value.path,
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
    async fn id(&mut self, delegate: Resource<ExtensionWorktree>) -> wasmtime::Result<u64> {
        latest::HostWorktree::id(self, delegate).await
    }

    async fn root_path(
        &mut self,
        delegate: Resource<ExtensionWorktree>,
    ) -> wasmtime::Result<String> {
        latest::HostWorktree::root_path(self, delegate).await
    }

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

impl HostKeyValueStore for WasmState {
    async fn insert(
        &mut self,
        _store: Resource<KeyValueStore>,
        _key: String,
        _value: String,
    ) -> wasmtime::Result<Result<(), String>> {
        Ok(Ok(()))
    }

    async fn drop(&mut self, _store: Resource<KeyValueStore>) -> wasmtime::Result<()> {
        Ok(())
    }
}

impl ExtensionImports for WasmState {
    async fn get_settings(
        &mut self,
        location: Option<SettingsLocation>,
        category: String,
        key: Option<String>,
    ) -> wasmtime::Result<Result<String, String>> {
        latest::ExtensionImports::get_settings(self, location.map(Into::into), category, key).await
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
