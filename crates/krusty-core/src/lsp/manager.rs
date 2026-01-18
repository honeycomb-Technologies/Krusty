//! LSP Manager - manages multiple language servers

use anyhow::{anyhow, Result};
use lsp_types::{PublishDiagnosticsParams, Uri};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};

use super::builtin::BuiltinLsp;
use super::client::LspClient;
use super::diagnostics::DiagnosticsCache;

/// Time to wait for LSP diagnostics after file changes (milliseconds)
///
/// LSP servers typically respond within 100-200ms for syntax errors.
/// Some servers (like TypeScript) may take longer for complex analysis.
pub const DIAGNOSTICS_WAIT_MS: u64 = 150;

/// Entry in the server registry with priority
#[derive(Debug, Clone)]
pub struct ServerEntry {
    pub server_id: String,
    pub priority: u8,
}

/// Suggestion for an LSP server to install
#[derive(Debug, Clone)]
pub enum LspSuggestion {
    /// A built-in LSP that can be auto-downloaded
    Builtin(&'static BuiltinLsp),
    /// A Zed extension to install
    Extension(String),
    /// Unknown language, no suggestion
    None,
}

/// Information about a missing LSP for a file extension
#[derive(Debug, Clone)]
pub struct MissingLspInfo {
    pub extension: String,
    pub language: String,
    pub suggested: LspSuggestion,
}

/// Convert a Path to an lsp_types::Uri
fn path_to_uri(path: &Path) -> Result<Uri> {
    let url = url::Url::from_file_path(path).map_err(|_| anyhow!("Invalid path: {:?}", path))?;
    Uri::from_str(url.as_str()).map_err(|e| anyhow!("Invalid URI: {}", e))
}

/// Server configuration for a language
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
}

/// LSP Manager - manages multiple language servers
pub struct LspManager {
    root_path: PathBuf,
    configs: RwLock<HashMap<String, ServerConfig>>,
    clients: RwLock<HashMap<String, Arc<LspClient>>>,
    /// Maps file extension to list of server options, sorted by priority (highest first)
    extension_map: RwLock<HashMap<String, Vec<ServerEntry>>>,
    language_ids: RwLock<HashMap<String, String>>, // extension -> language id
    file_versions: RwLock<HashMap<Uri, i32>>,      // Track document version per file
    open_files: RwLock<std::collections::HashSet<Uri>>, // Track which files are open
    diagnostics: Arc<DiagnosticsCache>,
    diagnostics_tx: mpsc::UnboundedSender<PublishDiagnosticsParams>,
    /// Keep receiver alive so sender doesn't error
    _diagnostics_rx: RwLock<Option<mpsc::UnboundedReceiver<PublishDiagnosticsParams>>>,
}

impl LspManager {
    /// Create a new LSP manager
    pub fn new(root_path: PathBuf) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();

        Self {
            root_path,
            configs: RwLock::new(HashMap::new()),
            clients: RwLock::new(HashMap::new()),
            extension_map: RwLock::new(HashMap::new()),
            language_ids: RwLock::new(HashMap::new()),
            file_versions: RwLock::new(HashMap::new()),
            open_files: RwLock::new(std::collections::HashSet::new()),
            diagnostics: Arc::new(DiagnosticsCache::new()),
            diagnostics_tx: tx,
            _diagnostics_rx: RwLock::new(Some(rx)),
        }
    }

    /// Get or spawn server for a file extension
    /// Tries servers in priority order (highest first)
    pub async fn get_client_for_extension(
        &self,
        extension: &str,
    ) -> Result<Option<Arc<LspClient>>> {
        // Look up server entries for extension (sorted by priority)
        let entries = {
            let ext_map = self.extension_map.read().await;
            ext_map.get(extension).cloned()
        };

        let entries = match entries {
            Some(e) if !e.is_empty() => e,
            _ => {
                debug!("No LSP server registered for extension: {}", extension);
                return Ok(None);
            }
        };

        // Try servers in priority order
        for entry in &entries {
            // Check if already running
            {
                let clients = self.clients.read().await;
                if let Some(client) = clients.get(&entry.server_id) {
                    return Ok(Some(client.clone()));
                }
            }

            // Try to spawn this server
            match self.spawn_server(&entry.server_id).await {
                Ok(Some(client)) => return Ok(Some(client)),
                Ok(None) => continue, // Config missing, try next
                Err(e) => {
                    warn!(
                        "Failed to spawn LSP {} for .{}, trying next: {}",
                        entry.server_id, extension, e
                    );
                    continue;
                }
            }
        }

        // All servers failed
        Ok(None)
    }

    /// Check if any LSP is registered for an extension
    pub async fn has_lsp_for_extension(&self, extension: &str) -> bool {
        let ext_map = self.extension_map.read().await;
        ext_map
            .get(extension)
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }

    /// Spawn a server by name
    async fn spawn_server(&self, name: &str) -> Result<Option<Arc<LspClient>>> {
        let config = {
            let configs = self.configs.read().await;
            configs.get(name).cloned()
        };

        let config = match config {
            Some(c) => c,
            None => return Err(anyhow!("No config for server: {}", name)),
        };

        info!(
            "Spawning LSP server: {} (command: {} {:?})",
            name, config.command, config.args
        );

        let args: Vec<&str> = config.args.iter().map(|s| s.as_str()).collect();

        let client = LspClient::spawn(
            &config.name,
            &config.command,
            &args,
            &self.root_path,
            self.diagnostics.clone(),
        )
        .await?;

        let client = Arc::new(client);

        // Start receive loop BEFORE initialize (otherwise request/response deadlocks)
        let rx = client.clone().start_receive_loop();
        self.forward_diagnostics(rx);

        // Now initialize (receive loop is running to handle the response)
        let root_uri = path_to_uri(&self.root_path)?;
        client.initialize(root_uri).await?;

        // Store client
        self.clients
            .write()
            .await
            .insert(name.to_string(), client.clone());

        Ok(Some(client))
    }

    /// Forward diagnostics from a client to the main channel
    fn forward_diagnostics(&self, mut rx: mpsc::UnboundedReceiver<PublishDiagnosticsParams>) {
        let tx = self.diagnostics_tx.clone();

        tokio::spawn(async move {
            while let Some(params) = rx.recv().await {
                let _ = tx.send(params);
            }
        });
    }

    /// Notify that a file changed (or was opened for the first time)
    pub async fn did_change(&self, path: &Path, text: &str) -> Result<()> {
        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        if let Some(client) = self.get_client_for_extension(extension).await? {
            let uri = path_to_uri(path)?;

            // Check if file is already open
            let is_open = self.open_files.read().await.contains(&uri);

            if is_open {
                // File already open - send didChange
                let version = {
                    let mut versions = self.file_versions.write().await;
                    let version = versions.entry(uri.clone()).or_insert(0);
                    *version += 1;
                    *version
                };
                client.did_change(uri, version, text).await?;
            } else {
                // File not open - send didOpen
                let language_id = self
                    .language_ids
                    .read()
                    .await
                    .get(extension)
                    .cloned()
                    .unwrap_or_else(|| extension.to_string());

                info!(
                    "Opening file in LSP: {:?} (language: {})",
                    path, language_id
                );
                client.did_open(uri.clone(), &language_id, 1, text).await?;

                // Track as open
                self.open_files.write().await.insert(uri.clone());
                self.file_versions.write().await.insert(uri, 1);
            }
        }

        Ok(())
    }

    /// Get diagnostics cache
    pub fn diagnostics_cache(&self) -> Arc<DiagnosticsCache> {
        self.diagnostics.clone()
    }

    /// Touch a file: open it in LSP and optionally wait for diagnostics
    ///
    /// After modifying a file, call this to trigger LSP analysis and
    /// get fresh diagnostics for tool output injection.
    ///
    /// Returns `Some(MissingLspInfo)` if no LSP is available for this file type,
    /// allowing the caller to prompt the user to install one.
    pub async fn touch_file(
        &self,
        path: &Path,
        wait_for_diagnostics: bool,
    ) -> Result<Option<MissingLspInfo>> {
        info!("touch_file called for: {:?}", path);

        // Check if we have an LSP for this file extension
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string();

        let has_lsp = self.has_lsp_for_extension(&ext).await;

        if !has_lsp {
            // No LSP available - return suggestion for installation
            let suggestion = suggest_lsp_for_extension(&ext);
            // Only suggest if we actually have something to offer
            if !matches!(suggestion, LspSuggestion::None) {
                return Ok(Some(MissingLspInfo {
                    extension: ext.clone(),
                    language: extension_to_language(&ext),
                    suggested: suggestion,
                }));
            }
            // Unknown file type, no suggestion - just continue without LSP
            return Ok(None);
        }

        // Read file content and notify LSP
        let text = tokio::fs::read_to_string(path).await?;
        self.did_change(path, &text).await?;

        if wait_for_diagnostics {
            // Wait a short time for diagnostics to be processed
            tokio::time::sleep(Duration::from_millis(DIAGNOSTICS_WAIT_MS)).await;
        }

        Ok(None)
    }

    /// Get diagnostics for a specific file path
    pub fn get_file_diagnostics(&self, path: &Path) -> Option<String> {
        self.diagnostics.format_for_file(path)
    }

    /// Register a language server from an extension's command
    ///
    /// Priority determines order when multiple servers support the same extension:
    /// - 100 = builtin (preferred, tested/stable)
    /// - 50 = extension (fallback)
    pub async fn register_from_extension(
        &self,
        server_id: &str,
        command: crate::extensions::wasm_host::Command,
        file_extensions: Vec<String>,
        priority: u8,
    ) -> Result<()> {
        let config = ServerConfig {
            name: server_id.to_string(),
            command: command.command,
            args: command.args,
        };

        info!(
            "Registered LSP '{}' (priority={}) for {:?}",
            server_id, priority, file_extensions
        );

        // Store config
        self.configs
            .write()
            .await
            .insert(server_id.to_string(), config);

        // Update extension map with priority-sorted entries
        let mut ext_map = self.extension_map.write().await;
        let mut lang_ids = self.language_ids.write().await;

        for ext in &file_extensions {
            let entry = ServerEntry {
                server_id: server_id.to_string(),
                priority,
            };

            let entries = ext_map.entry(ext.clone()).or_insert_with(Vec::new);

            // Check if this server is already registered for this extension
            if !entries.iter().any(|e| e.server_id == server_id) {
                entries.push(entry);
                // Sort by priority descending (highest first)
                entries.sort_by(|a, b| b.priority.cmp(&a.priority));

                if entries.len() > 1 {
                    info!(
                        "Extension .{} now has {} LSP options: {:?}",
                        ext,
                        entries.len(),
                        entries.iter().map(|e| &e.server_id).collect::<Vec<_>>()
                    );
                }
            }

            // Map common extensions to LSP language IDs
            let lang_id = match ext.as_str() {
                "rs" => "rust",
                "toml" => "toml",
                "py" => "python",
                "js" => "javascript",
                "ts" => "typescript",
                "tsx" => "typescriptreact",
                "jsx" => "javascriptreact",
                "go" => "go",
                "c" => "c",
                "cpp" | "cc" | "cxx" => "cpp",
                "h" | "hpp" => "cpp",
                "java" => "java",
                "rb" => "ruby",
                "php" => "php",
                "swift" => "swift",
                "kt" => "kotlin",
                "cs" => "csharp",
                "lua" => "lua",
                "sh" | "bash" => "shellscript",
                "json" => "json",
                "yaml" | "yml" => "yaml",
                "md" => "markdown",
                "html" => "html",
                "css" => "css",
                "scss" => "scss",
                "sql" => "sql",
                other => other,
            };
            lang_ids.insert(ext.clone(), lang_id.to_string());
        }
        Ok(())
    }

    /// Register a built-in language server with a specific binary path
    pub async fn register_builtin_with_path(
        &self,
        builtin: &super::builtin::BuiltinLsp,
        bin_path: &std::path::Path,
    ) -> Result<()> {
        self.register_from_extension(
            builtin.id,
            builtin.to_command_with_path(bin_path),
            builtin.file_extensions(),
            100, // Builtins get highest priority (tested/stable)
        )
        .await
    }

    /// Register all built-in servers, downloading if needed
    pub async fn register_all_builtins(&self, downloader: &super::downloader::LspDownloader) {
        for builtin in super::builtin::BUILTIN_LSPS {
            // Try to get binary (downloads if needed)
            match downloader.ensure_available(builtin).await {
                Ok(bin_path) => {
                    info!("Registering built-in LSP: {} ({:?})", builtin.id, bin_path);
                    if let Err(e) = self.register_builtin_with_path(builtin, &bin_path).await {
                        tracing::warn!("Failed to register built-in LSP {}: {}", builtin.id, e);
                    }
                }
                Err(e) => {
                    debug!("LSP {} not available: {}", builtin.id, e);
                }
            }
        }
    }

    /// Check the health status of all running LSP servers
    ///
    /// Returns a map of server_id -> is_healthy for all registered clients.
    pub async fn health_check(&self) -> HashMap<String, bool> {
        let clients = self.clients.read().await;
        let mut status = HashMap::new();

        for (name, client) in clients.iter() {
            let healthy = client.is_healthy();
            if !healthy {
                warn!("LSP server {} is unhealthy", name);
            }
            status.insert(name.clone(), healthy);
        }

        status
    }

    /// Restart an unhealthy LSP server
    ///
    /// Stops the existing server (if any) and spawns a fresh instance.
    /// Returns Ok(Some(client)) if restart succeeded, Ok(None) if no config exists.
    pub async fn restart_server(&self, server_id: &str) -> Result<Option<Arc<LspClient>>> {
        info!("Restarting LSP server: {}", server_id);

        // Remove existing client (this will drop it and kill the process)
        {
            let mut clients = self.clients.write().await;
            if let Some(old_client) = clients.remove(server_id) {
                info!(
                    "Removed old LSP client {} (was healthy: {})",
                    server_id,
                    old_client.is_healthy()
                );
            }
        }

        // Spawn a new instance
        self.spawn_server(server_id).await
    }

    /// Get list of currently running LSP server names
    pub async fn running_servers(&self) -> Vec<String> {
        self.clients.read().await.keys().cloned().collect()
    }
}

/// Map file extension to human-readable language name
pub fn extension_to_language(ext: &str) -> String {
    match ext {
        "rs" => "Rust",
        "py" | "pyi" => "Python",
        "js" | "mjs" => "JavaScript",
        "ts" | "mts" => "TypeScript",
        "tsx" => "TypeScript React",
        "jsx" => "JavaScript React",
        "go" => "Go",
        "c" => "C",
        "cpp" | "cc" | "cxx" => "C++",
        "h" | "hpp" => "C/C++ Header",
        "java" => "Java",
        "rb" => "Ruby",
        "lua" => "Lua",
        "zig" => "Zig",
        "sh" | "bash" => "Shell",
        "json" => "JSON",
        "yaml" | "yml" => "YAML",
        "toml" => "TOML",
        "md" => "Markdown",
        "html" => "HTML",
        "css" => "CSS",
        "sql" => "SQL",
        other => other,
    }
    .to_string()
}

/// Suggest an LSP server to install for a file extension
pub fn suggest_lsp_for_extension(ext: &str) -> LspSuggestion {
    use super::builtin::BUILTIN_LSPS;

    // Check if a builtin covers this extension
    for builtin in BUILTIN_LSPS {
        if builtin.extensions.contains(&ext) {
            return LspSuggestion::Builtin(builtin);
        }
    }

    // Suggest Zed extensions for languages without builtins
    match ext {
        "ex" | "exs" => LspSuggestion::Extension("elixir".to_string()),
        "erl" | "hrl" => LspSuggestion::Extension("erlang".to_string()),
        "swift" => LspSuggestion::Extension("swift".to_string()),
        "kt" | "kts" => LspSuggestion::Extension("kotlin".to_string()),
        "scala" | "sc" => LspSuggestion::Extension("scala".to_string()),
        "r" | "R" => LspSuggestion::Extension("r".to_string()),
        "jl" => LspSuggestion::Extension("julia".to_string()),
        "nim" => LspSuggestion::Extension("nim".to_string()),
        "ml" | "mli" => LspSuggestion::Extension("ocaml".to_string()),
        "hs" => LspSuggestion::Extension("haskell".to_string()),
        "gleam" => LspSuggestion::Extension("gleam".to_string()),
        _ => LspSuggestion::None,
    }
}
