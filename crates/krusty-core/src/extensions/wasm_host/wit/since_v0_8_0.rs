//! WIT bindings for extension API v0.8.0 (latest)

use crate::extensions::types::WorktreeDelegate;
use crate::extensions::wasm_host::WasmState;
use anyhow::Result;
use semver::Version;
use std::sync::{Arc, OnceLock};
use wasmtime::component::{Linker, Resource};

pub const MIN_VERSION: Version = Version::new(0, 8, 0);

wasmtime::component::bindgen!({
    async: true,
    trappable_imports: true,
    path: "src/extensions/wit/since_v0.8.0",
    with: {
        "worktree": ExtensionWorktree,
    },
});

pub type ExtensionWorktree = Arc<dyn WorktreeDelegate>;

pub fn linker() -> &'static Linker<WasmState> {
    static LINKER: OnceLock<Linker<WasmState>> = OnceLock::new();
    LINKER.get_or_init(|| super::new_linker(Extension::add_to_linker))
}

// Host trait implementations

impl HostWorktree for WasmState {
    async fn id(&mut self, delegate: Resource<ExtensionWorktree>) -> wasmtime::Result<u64> {
        let delegate = self.table.get(&delegate)?;
        Ok(delegate.id())
    }

    async fn root_path(
        &mut self,
        delegate: Resource<ExtensionWorktree>,
    ) -> wasmtime::Result<String> {
        let delegate = self.table.get(&delegate)?;
        Ok(delegate.root_path())
    }

    async fn read_text_file(
        &mut self,
        delegate: Resource<ExtensionWorktree>,
        path: String,
    ) -> wasmtime::Result<Result<String, String>> {
        let delegate = self.table.get(&delegate)?;
        match delegate.read_text_file(&path).await {
            Ok(content) => Ok(Ok(content)),
            Err(e) => Ok(Err(e.to_string())),
        }
    }

    async fn shell_env(
        &mut self,
        delegate: Resource<ExtensionWorktree>,
    ) -> wasmtime::Result<EnvVars> {
        let delegate = self.table.get(&delegate)?;
        Ok(delegate.shell_env())
    }

    async fn which(
        &mut self,
        delegate: Resource<ExtensionWorktree>,
        binary_name: String,
    ) -> wasmtime::Result<Option<String>> {
        let delegate = self.table.get(&delegate)?;
        Ok(delegate.which(&binary_name))
    }

    async fn drop(&mut self, _delegate: Resource<ExtensionWorktree>) -> wasmtime::Result<()> {
        Ok(())
    }
}

impl HostProject for WasmState {
    async fn worktree_ids(&mut self, _project: Resource<Project>) -> wasmtime::Result<Vec<u64>> {
        Ok(vec![])
    }

    async fn drop(&mut self, _project: Resource<Project>) -> wasmtime::Result<()> {
        Ok(())
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
        _location: Option<SettingsLocation>,
        _category: String,
        _key: Option<String>,
    ) -> wasmtime::Result<Result<String, String>> {
        Ok(Ok("{}".to_string()))
    }

    async fn download_file(
        &mut self,
        url: String,
        path: String,
        file_type: DownloadedFileType,
    ) -> wasmtime::Result<Result<(), String>> {
        let dest_path = self
            .host
            .writeable_path_from_extension(&self.manifest.id, std::path::Path::new(&path));

        if let Some(parent) = dest_path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return Ok(Err(e.to_string()));
            }
        }

        let response = match self.host.http_client().get(&url).send().await {
            Ok(r) => r,
            Err(e) => return Ok(Err(e.to_string())),
        };

        if !response.status().is_success() {
            return Ok(Err(format!(
                "download failed with status {}",
                response.status()
            )));
        }

        let bytes = match response.bytes().await {
            Ok(b) => b,
            Err(e) => return Ok(Err(e.to_string())),
        };

        let result = match file_type {
            DownloadedFileType::Uncompressed => tokio::fs::write(&dest_path, &bytes)
                .await
                .map_err(|e| e.to_string()),
            DownloadedFileType::Gzip => {
                use flate2::read::GzDecoder;
                use std::io::Read;
                let mut decoder = GzDecoder::new(&bytes[..]);
                let mut decompressed = Vec::new();
                match decoder.read_to_end(&mut decompressed) {
                    Ok(_) => tokio::fs::write(&dest_path, &decompressed)
                        .await
                        .map_err(|e| e.to_string()),
                    Err(e) => Err(e.to_string()),
                }
            }
            DownloadedFileType::GzipTar => {
                use flate2::read::GzDecoder;
                use tar::Archive;
                let decoder = GzDecoder::new(&bytes[..]);
                let mut archive = Archive::new(decoder);
                archive.unpack(&dest_path).map_err(|e| e.to_string())
            }
            DownloadedFileType::Zip => {
                use std::io::Cursor;
                let reader = Cursor::new(&bytes);
                match zip::ZipArchive::new(reader) {
                    Ok(mut archive) => archive.extract(&dest_path).map_err(|e| e.to_string()),
                    Err(e) => Err(e.to_string()),
                }
            }
        };

        Ok(result)
    }

    async fn make_file_executable(&mut self, path: String) -> wasmtime::Result<Result<(), String>> {
        let full_path = self
            .host
            .writeable_path_from_extension(&self.manifest.id, std::path::Path::new(&path));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = match tokio::fs::metadata(&full_path).await {
                Ok(m) => m,
                Err(e) => return Ok(Err(e.to_string())),
            };
            let mut permissions = metadata.permissions();
            permissions.set_mode(permissions.mode() | 0o111);
            match tokio::fs::set_permissions(&full_path, permissions).await {
                Ok(_) => Ok(Ok(())),
                Err(e) => Ok(Err(e.to_string())),
            }
        }

        #[cfg(not(unix))]
        Ok(Ok(()))
    }

    async fn set_language_server_installation_status(
        &mut self,
        _server_name: String,
        _status: LanguageServerInstallationStatus,
    ) -> wasmtime::Result<()> {
        Ok(())
    }
}

// Stub implementations for interfaces

impl zed::extension::github::Host for WasmState {
    async fn latest_github_release(
        &mut self,
        repo: String,
        options: zed::extension::github::GithubReleaseOptions,
    ) -> wasmtime::Result<Result<zed::extension::github::GithubRelease, String>> {
        tracing::debug!(
            "GitHub API: latest_github_release called for repo '{}'",
            repo
        );
        match crate::extensions::github::latest_github_release(
            &repo,
            options.require_assets,
            options.pre_release,
            self.host.http_client(),
        )
        .await
        {
            Ok(release) => {
                tracing::debug!(
                    "GitHub API: found release {} with {} assets",
                    release.tag_name,
                    release.assets.len()
                );
                Ok(Ok(zed::extension::github::GithubRelease {
                    version: release.tag_name,
                    assets: release
                        .assets
                        .into_iter()
                        .map(|a| zed::extension::github::GithubReleaseAsset {
                            name: a.name,
                            download_url: a.browser_download_url,
                        })
                        .collect(),
                }))
            }
            Err(e) => {
                tracing::warn!("GitHub API error for '{}': {}", repo, e);
                Ok(Err(e.to_string()))
            }
        }
    }

    async fn github_release_by_tag_name(
        &mut self,
        repo: String,
        tag: String,
    ) -> wasmtime::Result<Result<zed::extension::github::GithubRelease, String>> {
        match crate::extensions::github::get_release_by_tag_name(
            &repo,
            &tag,
            self.host.http_client(),
        )
        .await
        {
            Ok(release) => Ok(Ok(zed::extension::github::GithubRelease {
                version: release.tag_name,
                assets: release
                    .assets
                    .into_iter()
                    .map(|a| zed::extension::github::GithubReleaseAsset {
                        name: a.name,
                        download_url: a.browser_download_url,
                    })
                    .collect(),
            })),
            Err(e) => Ok(Err(e.to_string())),
        }
    }
}

impl zed::extension::platform::Host for WasmState {
    async fn current_platform(
        &mut self,
    ) -> wasmtime::Result<(
        zed::extension::platform::Os,
        zed::extension::platform::Architecture,
    )> {
        #[cfg(target_os = "macos")]
        let os = zed::extension::platform::Os::Mac;
        #[cfg(target_os = "linux")]
        let os = zed::extension::platform::Os::Linux;
        #[cfg(target_os = "windows")]
        let os = zed::extension::platform::Os::Windows;
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        let os = zed::extension::platform::Os::Linux;

        #[cfg(target_arch = "x86_64")]
        let arch = zed::extension::platform::Architecture::X8664;
        #[cfg(target_arch = "aarch64")]
        let arch = zed::extension::platform::Architecture::Aarch64;
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        let arch = zed::extension::platform::Architecture::X8664;

        Ok((os, arch))
    }
}

impl zed::extension::nodejs::Host for WasmState {
    async fn node_binary_path(&mut self) -> wasmtime::Result<Result<String, String>> {
        // Return bun path - it's node-compatible for running JS
        match self.host.bun_runtime().binary_path().await {
            Ok(path) => Ok(Ok(path.to_string_lossy().into_owned())),
            Err(e) => Ok(Err(e.to_string())),
        }
    }

    async fn npm_package_latest_version(
        &mut self,
        package: String,
    ) -> wasmtime::Result<Result<String, String>> {
        match self
            .host
            .bun_runtime()
            .npm_package_latest_version(&package)
            .await
        {
            Ok(version) => Ok(Ok(version)),
            Err(e) => Ok(Err(e.to_string())),
        }
    }

    async fn npm_package_installed_version(
        &mut self,
        package: String,
    ) -> wasmtime::Result<Result<Option<String>, String>> {
        let work_dir = self.work_dir();
        match self
            .host
            .bun_runtime()
            .npm_package_installed_version(&work_dir, &package)
            .await
        {
            Ok(version) => Ok(Ok(version)),
            Err(e) => Ok(Err(e.to_string())),
        }
    }

    async fn npm_install_package(
        &mut self,
        package: String,
        version: String,
    ) -> wasmtime::Result<Result<(), String>> {
        let work_dir = self.work_dir();
        match self
            .host
            .bun_runtime()
            .npm_install_packages(&work_dir, &[(&package, &version)])
            .await
        {
            Ok(()) => Ok(Ok(())),
            Err(e) => Ok(Err(e.to_string())),
        }
    }
}

impl zed::extension::http_client::Host for WasmState {
    async fn fetch(
        &mut self,
        request: zed::extension::http_client::HttpRequest,
    ) -> wasmtime::Result<Result<zed::extension::http_client::HttpResponse, String>> {
        let client = self.host.http_client();

        let method = match request.method {
            zed::extension::http_client::HttpMethod::Get => reqwest::Method::GET,
            zed::extension::http_client::HttpMethod::Post => reqwest::Method::POST,
            zed::extension::http_client::HttpMethod::Put => reqwest::Method::PUT,
            zed::extension::http_client::HttpMethod::Delete => reqwest::Method::DELETE,
            zed::extension::http_client::HttpMethod::Head => reqwest::Method::HEAD,
            zed::extension::http_client::HttpMethod::Options => reqwest::Method::OPTIONS,
            zed::extension::http_client::HttpMethod::Patch => reqwest::Method::PATCH,
        };

        let mut req = client.request(method, &request.url);

        for (key, value) in &request.headers {
            req = req.header(key, value);
        }

        if let Some(body) = request.body {
            req = req.body(body);
        }

        match req.send().await {
            Ok(response) => {
                let headers: Vec<(String, String)> = response
                    .headers()
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                    .collect();

                match response.bytes().await {
                    Ok(body) => Ok(Ok(zed::extension::http_client::HttpResponse {
                        headers,
                        body: body.to_vec(),
                    })),
                    Err(e) => Ok(Err(e.to_string())),
                }
            }
            Err(e) => Ok(Err(e.to_string())),
        }
    }

    async fn fetch_stream(
        &mut self,
        _request: zed::extension::http_client::HttpRequest,
    ) -> wasmtime::Result<Result<Resource<zed::extension::http_client::HttpResponseStream>, String>>
    {
        // Streaming not needed for LSP binary downloads
        Ok(Err("HTTP streaming not implemented".to_string()))
    }
}

impl zed::extension::http_client::HostHttpResponseStream for WasmState {
    async fn next_chunk(
        &mut self,
        _stream: Resource<zed::extension::http_client::HttpResponseStream>,
    ) -> wasmtime::Result<Result<Option<Vec<u8>>, String>> {
        Ok(Ok(None))
    }

    async fn drop(
        &mut self,
        _stream: Resource<zed::extension::http_client::HttpResponseStream>,
    ) -> wasmtime::Result<()> {
        Ok(())
    }
}

impl zed::extension::process::Host for WasmState {
    async fn run_command(
        &mut self,
        command: zed::extension::process::Command,
    ) -> wasmtime::Result<Result<zed::extension::process::Output, String>> {
        use tokio::process::Command as TokioCommand;

        match TokioCommand::new(&command.command)
            .args(&command.args)
            .envs(command.env)
            .output()
            .await
        {
            Ok(output) => Ok(Ok(zed::extension::process::Output {
                status: output.status.code(),
                stdout: output.stdout,
                stderr: output.stderr,
            })),
            Err(e) => Ok(Err(e.to_string())),
        }
    }
}

impl zed::extension::slash_command::Host for WasmState {}

impl zed::extension::context_server::Host for WasmState {}

impl zed::extension::common::Host for WasmState {}

impl zed::extension::dap::Host for WasmState {
    async fn resolve_tcp_template(
        &mut self,
        _template: zed::extension::dap::TcpArgumentsTemplate,
    ) -> wasmtime::Result<Result<zed::extension::dap::TcpArguments, String>> {
        Ok(Err("DAP not implemented".to_string()))
    }
}
