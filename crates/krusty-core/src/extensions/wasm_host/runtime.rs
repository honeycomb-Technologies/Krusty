use crate::extensions::{bun_runtime::BunRuntime, types::*, ExtensionManifest};
use anyhow::{anyhow, bail, Context as _, Result};
use futures::{
    channel::{
        mpsc::{self, UnboundedSender},
        oneshot,
    },
    future::BoxFuture,
    FutureExt, StreamExt as _,
};
use std::{
    path::{Path, PathBuf},
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use wasmtime::{component::Component, Store};
use wasmtime_wasi::{self as wasi};

use super::{
    engine::wasm_engine,
    state::{ExtensionCall, WasmState, IS_WASM_THREAD},
    wit,
};

/// The WASM extension host - manages loading and running WASM extensions
pub struct WasmHost {
    engine: wasmtime::Engine,
    http_client: reqwest::Client,
    /// Directory where extensions can write files (separate from install directory)
    pub work_dir: PathBuf,
    bun_runtime: BunRuntime,
    _epoch_task: tokio::task::JoinHandle<()>,
}

/// A loaded WASM extension
#[derive(Clone)]
pub struct WasmExtension {
    tx: UnboundedSender<ExtensionCall>,
    pub manifest: Arc<ExtensionManifest>,
    _task: Arc<tokio::task::JoinHandle<()>>,
}

impl Drop for WasmExtension {
    fn drop(&mut self) {
        self.tx.close_channel();
    }
}

impl WasmHost {
    /// Create a new WASM host
    pub fn new(http_client: reqwest::Client, extensions_dir: PathBuf) -> Arc<Self> {
        let engine = wasm_engine();

        let epoch_engine = engine.clone();
        let epoch_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            loop {
                interval.tick().await;
                epoch_engine.increment_epoch();
            }
        });

        let work_dir = extensions_dir.join("work");
        let data_dir = extensions_dir
            .parent()
            .unwrap_or(&extensions_dir)
            .to_path_buf();
        let bun_runtime = BunRuntime::new(http_client.clone(), data_dir);

        Arc::new(Self {
            engine,
            http_client,
            work_dir,
            bun_runtime,
            _epoch_task: epoch_task,
        })
    }

    pub fn bun_runtime(&self) -> &BunRuntime {
        &self.bun_runtime
    }

    pub fn http_client(&self) -> &reqwest::Client {
        &self.http_client
    }

    pub fn writeable_path_from_extension(&self, extension_id: &str, path: &Path) -> PathBuf {
        let normalized = normalize_path(path);
        let extension_work_dir = self.work_dir.join(extension_id);
        extension_work_dir.join(normalized)
    }

    /// Discover and load every valid Zed-compatible extension directory under
    /// `root`. One broken extension is isolated and reported without preventing
    /// the remaining extensions from starting.
    pub async fn load_extensions_from_root(
        self: &Arc<Self>,
        root: &Path,
    ) -> (Vec<WasmExtension>, Vec<(PathBuf, String)>) {
        if !root.is_dir() {
            return (Vec::new(), Vec::new());
        }

        let mut directories = Vec::new();
        let mut diagnostics = Vec::new();
        match tokio::fs::read_dir(root).await {
            Ok(mut entries) => loop {
                match entries.next_entry().await {
                    Ok(Some(entry)) => {
                        let path = entry.path();
                        if path.is_dir() && path.join("extension.toml").is_file() {
                            directories.push(path);
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        diagnostics.push((root.to_path_buf(), error.to_string()));
                        break;
                    }
                }
            },
            Err(error) => {
                diagnostics.push((root.to_path_buf(), error.to_string()));
                return (Vec::new(), diagnostics);
            }
        }
        directories.sort();

        let mut loaded = Vec::new();
        for directory in directories {
            match self.load_extension_from_dir(&directory).await {
                Ok(extension) => loaded.push(extension),
                Err(error) => diagnostics.push((directory, format!("{error:#}"))),
            }
        }
        (loaded, diagnostics)
    }

    /// Load an extension from a directory containing extension.toml and *.wasm
    pub async fn load_extension_from_dir(
        self: &Arc<Self>,
        extension_dir: &Path,
    ) -> Result<WasmExtension> {
        let manifest_path = extension_dir.join("extension.toml");
        let manifest_content = tokio::fs::read_to_string(&manifest_path)
            .await
            .context("Failed to read extension.toml")?;
        let manifest: ExtensionManifest =
            toml::from_str(&manifest_content).context("Failed to parse extension.toml")?;

        let wasm_filename = format!("{}.wasm", manifest.id);
        let wasm_path = extension_dir.join(&wasm_filename);

        if !wasm_path.exists() {
            let fallback = extension_dir.join("extension.wasm");
            if fallback.exists() {
                let extension_work_dir = self.work_dir.join(&manifest.id);
                tokio::fs::create_dir_all(&extension_work_dir).await?;
                return self
                    .load_extension(fallback, manifest, extension_work_dir)
                    .await;
            }
            bail!("WASM file not found: {:?} or extension.wasm", wasm_path);
        }

        let extension_work_dir = self.work_dir.join(&manifest.id);
        tokio::fs::create_dir_all(&extension_work_dir).await?;

        self.load_extension(wasm_path, manifest, extension_work_dir)
            .await
    }

    pub async fn load_extension(
        self: &Arc<Self>,
        wasm_path: PathBuf,
        manifest: ExtensionManifest,
        extension_work_dir: PathBuf,
    ) -> Result<WasmExtension> {
        let wasm_bytes = tokio::fs::read(&wasm_path)
            .await
            .context("Failed to read WASM file")?;

        let zed_api_version =
            crate::extensions::manifest::parse_wasm_extension_version(&manifest.id, &wasm_bytes)?;

        let component = Component::from_binary(&self.engine, &wasm_bytes)
            .context("Failed to compile WASM component")?;

        let manifest = Arc::new(manifest);
        let (tx, mut rx) = mpsc::unbounded::<ExtensionCall>();

        let host = self.clone();
        let ext_manifest = manifest.clone();
        let version_for_check = zed_api_version;

        let task = tokio::spawn(async move {
            IS_WASM_THREAD.with(|flag| flag.store(true, Ordering::SeqCst));

            let extension_work_dir_str = extension_work_dir.to_string_lossy().into_owned();
            tracing::debug!(
                "Setting up WASI for extension {} with work_dir: {}",
                ext_manifest.id,
                extension_work_dir_str
            );

            match std::fs::metadata(&extension_work_dir) {
                Ok(meta) => {
                    tracing::debug!("Work dir exists, is_dir: {}", meta.is_dir());
                }
                Err(e) => {
                    tracing::error!("Work dir not accessible: {}", e);
                }
            }

            let mut wasi_builder = wasi::WasiCtxBuilder::new();
            wasi_builder
                .inherit_stdio()
                .env("PWD", &extension_work_dir_str)
                .env("RUST_BACKTRACE", "full")
                .env("HOME", std::env::var("HOME").unwrap_or_default());

            if let Err(e) = wasi_builder.preopened_dir(
                &extension_work_dir,
                ".",
                wasi::DirPerms::all(),
                wasi::FilePerms::all(),
            ) {
                tracing::error!("Failed to preopen '.' for {}: {}", ext_manifest.id, e);
            }

            if let Err(e) = wasi_builder.preopened_dir(
                &extension_work_dir,
                &extension_work_dir_str,
                wasi::DirPerms::all(),
                wasi::FilePerms::all(),
            ) {
                tracing::error!(
                    "Failed to preopen absolute path for {}: {}",
                    ext_manifest.id,
                    e
                );
            }

            let wasi_ctx = wasi_builder.build();

            let mut store = Store::new(
                &host.engine,
                WasmState {
                    manifest: ext_manifest.clone(),
                    table: wasmtime::component::ResourceTable::new(),
                    ctx: wasi_ctx,
                    host: host.clone(),
                },
            );
            store.set_epoch_deadline(1);
            store.epoch_deadline_async_yield_and_update(1);

            let mut extension =
                match wit::Extension::instantiate_async(&mut store, &version_for_check, &component)
                    .await
                {
                    Ok(ext) => ext,
                    Err(e) => {
                        tracing::error!("Failed to instantiate extension: {:#}", e);
                        return;
                    }
                };

            tracing::info!(
                "Extension {} instantiated with API version {}",
                ext_manifest.id,
                version_for_check
            );

            if let Err(e) = extension.call_init_extension(&mut store).await {
                tracing::error!("Failed to initialize extension: {}", e);
                return;
            }

            while let Some(call) = rx.next().await {
                call(&mut extension, &mut store).await;
            }
        });

        Ok(WasmExtension {
            tx,
            manifest,
            _task: Arc::new(task),
        })
    }
}

impl WasmExtension {
    /// Call a method on the extension
    pub async fn call<T, Fn>(&self, f: Fn) -> Result<T>
    where
        T: 'static + Send,
        Fn: 'static
            + Send
            + for<'a> FnOnce(&'a mut wit::Extension, &'a mut Store<WasmState>) -> BoxFuture<'a, T>,
    {
        let (return_tx, return_rx) = oneshot::channel();
        self.tx
            .unbounded_send(Box::new(move |extension, store| {
                async {
                    let result = f(extension, store).await;
                    return_tx.send(result).ok();
                }
                .boxed()
            }))
            .map_err(|_| anyhow!("Extension {} channel has stopped", self.manifest.name))?;

        return_rx
            .await
            .with_context(|| format!("Extension {} channel dropped", self.manifest.name))
    }

    /// Get the language server command from the extension
    pub async fn language_server_command(
        &self,
        language_server_id: LanguageServerName,
        worktree: Arc<dyn WorktreeDelegate>,
    ) -> Result<super::Command> {
        self.call(|extension, store| {
            async move {
                let resource = store.data_mut().table.push(worktree)?;
                let command = extension
                    .call_language_server_command(store, &language_server_id, resource)
                    .await?
                    .map_err(|err| store.data().extension_error(err))?;
                Ok(command)
            }
            .boxed()
        })
        .await?
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut components = vec![];
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                components.pop();
            }
            std::path::Component::CurDir => {}
            c => components.push(c),
        }
    }
    components.iter().collect()
}
