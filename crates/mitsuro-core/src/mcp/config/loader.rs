use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tokio::io::AsyncReadExt;

use super::expansion::expand_env_var;
use super::types::{
    McpConfig, McpConfigSource, McpConnectionAuthority, McpPackageConfig, McpServerConfig,
    McpServerConfigRaw, McpServerOptions, McpServerOptionsRaw, RemoteMcpServer,
};

pub(super) const MAX_MCP_CONFIG_BYTES: usize = 1024 * 1024;
pub(super) const MAX_MCP_PACKAGE_FRAGMENTS: usize = 128;
pub(super) const MAX_MCP_SERVERS: usize = 1024;
pub(super) const MAX_MCP_STARTUP_TIMEOUT_MS: u64 = 2 * 60 * 1000;
pub(super) const MAX_MCP_TOOL_TIMEOUT_MS: u64 = 10 * 60 * 1000;

impl McpConfig {
    /// Load and merge `~/.mitsuro/mcp.json` with `<project>/.mcp.json`.
    ///
    /// Project declarations override global declarations by server name.
    pub async fn load(working_dir: &Path) -> Result<Self> {
        Self::load_with_global_path(working_dir, &crate::paths::config_dir().join("mcp.json")).await
    }

    /// Load using an explicit global path. This is public to support isolated
    /// embedders and deterministic tests without mutating the user's config.
    pub async fn load_with_global_path(working_dir: &Path, global_path: &Path) -> Result<Self> {
        Self::load_with_package_paths(working_dir, global_path, &[]).await
    }

    /// Load package-provided fragments as defaults, then apply global and
    /// project declarations in that order. Later package fragments override
    /// earlier package fragments, but never user-owned configuration.
    pub async fn load_with_package_paths(
        working_dir: &Path,
        global_path: &Path,
        package_paths: &[PathBuf],
    ) -> Result<Self> {
        let package_configs = package_paths
            .iter()
            .cloned()
            .map(|path| McpPackageConfig::new(path, McpConnectionAuthority::NONE))
            .collect::<Vec<_>>();
        Self::load_with_package_configs(working_dir, global_path, &package_configs).await
    }

    /// Load package fragments together with the exact process/network grant
    /// attached to each installed package descriptor.
    pub async fn load_with_package_configs(
        working_dir: &Path,
        global_path: &Path,
        package_configs: &[McpPackageConfig],
    ) -> Result<Self> {
        if package_configs.len() > MAX_MCP_PACKAGE_FRAGMENTS {
            anyhow::bail!(
                "MCP configuration exceeds the package-fragment limit of {}",
                MAX_MCP_PACKAGE_FRAGMENTS
            );
        }
        let project_path = working_dir.join(".mcp.json");
        let global = read_optional_config(global_path).await?;
        let project = read_optional_config(&project_path).await?;

        let mut merged = McpConfig::default();
        for package_config in package_configs {
            if let Some(package) = read_optional_config(&package_config.path).await? {
                for (name, server) in package.mcp_servers {
                    merged
                        .package_server_authorities
                        .insert(name.clone(), package_config.authority);
                    merged.mcp_servers.insert(name, server);
                }
                ensure_server_limit(&merged, &package_config.path)?;
            }
        }

        let mut trusted_global_locals = HashSet::new();
        let mut global_servers = HashSet::new();
        if let Some(global) = global {
            for (name, server) in global.mcp_servers {
                if matches!(server, McpServerConfigRaw::Local { .. }) {
                    trusted_global_locals.insert(name.clone());
                }
                global_servers.insert(name.clone());
                merged.package_server_authorities.remove(&name);
                merged.mcp_servers.insert(name, server);
            }
            ensure_server_limit(&merged, global_path)?;
        }
        let mut project_servers = HashSet::new();

        if let Some(project) = project {
            for (name, server) in project.mcp_servers {
                trusted_global_locals.remove(&name);
                project_servers.insert(name.clone());
                merged.package_server_authorities.remove(&name);
                merged.mcp_servers.insert(name, server);
            }
            ensure_server_limit(&merged, &project_path)?;
        }

        merged.auto_connect_local_servers = trusted_global_locals;
        merged.project_servers = project_servers;
        merged.global_servers = global_servers;

        tracing::info!(
            global_path = %global_path.display(),
            project_path = %project_path.display(),
            package_fragments = package_configs.len(),
            servers = merged.mcp_servers.len(),
            "Loaded merged MCP configuration"
        );

        for name in &merged.project_servers {
            if matches!(
                merged.mcp_servers.get(name),
                Some(McpServerConfigRaw::Local { .. })
            ) {
                tracing::warn!(
                    server = name,
                    "Project MCP stdio server is disconnected until explicitly trusted and connected"
                );
            }
        }

        Ok(merged)
    }

    /// Get resolved server configurations.
    pub async fn servers(&self) -> HashMap<String, McpServerConfig> {
        let mut result = HashMap::new();
        for (name, raw) in &self.mcp_servers {
            let source = if self.project_servers.contains(name) {
                McpConfigSource::Project
            } else if self.global_servers.contains(name) {
                McpConfigSource::Global
            } else {
                McpConfigSource::Package
            };
            let trusted_global_local = self.auto_connect_local_servers.contains(name);
            let authority = match source {
                McpConfigSource::Global => McpConnectionAuthority::FULL,
                McpConfigSource::Project => McpConnectionAuthority::NONE,
                McpConfigSource::Package => self
                    .package_server_authorities
                    .get(name)
                    .copied()
                    .unwrap_or(McpConnectionAuthority::NONE),
            };

            let config = match raw {
                McpServerConfigRaw::Local {
                    command,
                    args,
                    env,
                    cwd,
                    options,
                } => {
                    let mut expanded_env = HashMap::new();
                    for (key, value) in env {
                        expanded_env.insert(
                            key.clone(),
                            expand_env_var(value, trusted_global_local).await,
                        );
                    }
                    let auto_connect = trusted_global_local
                        && options.auto_connect.unwrap_or(true)
                        && options.enabled;
                    McpServerConfig::Local {
                        command: command.clone(),
                        args: args.clone(),
                        env: expanded_env,
                        cwd: cwd.clone(),
                        options: resolve_options(options, source, auto_connect, authority),
                    }
                }
                McpServerConfigRaw::Remote {
                    url,
                    transport,
                    authorization_token,
                    bearer_token_env_var,
                    oauth,
                    headers,
                    env_headers,
                    options,
                    ..
                } => {
                    let trusted = source == McpConfigSource::Global;
                    let mut resolved_headers = HashMap::new();
                    for (key, value) in headers {
                        resolved_headers.insert(key.clone(), expand_env_var(value, trusted).await);
                    }
                    if trusted {
                        for (header_name, env_name) in env_headers {
                            match std::env::var(env_name) {
                                Ok(value) => {
                                    resolved_headers.insert(header_name.clone(), value);
                                }
                                Err(_) => tracing::warn!(
                                    server = name,
                                    header = header_name,
                                    env_var = env_name,
                                    "MCP environment-backed header is unavailable"
                                ),
                            }
                        }
                    } else if !env_headers.is_empty() {
                        tracing::warn!(
                            server = name,
                            source = ?source,
                            "Ignoring environment-backed headers from untrusted MCP configuration"
                        );
                    }

                    let env_token = if trusted {
                        bearer_token_env_var.as_ref().and_then(|env_name| {
                            match std::env::var(env_name) {
                                Ok(value) => Some(value),
                                Err(_) => {
                                    tracing::warn!(
                                        server = name,
                                        env_var = env_name,
                                        "MCP bearer-token environment variable is unavailable"
                                    );
                                    None
                                }
                            }
                        })
                    } else {
                        if bearer_token_env_var.is_some() {
                            tracing::warn!(
                                server = name,
                                source = ?source,
                                "Ignoring bearer-token environment variable from untrusted MCP configuration"
                            );
                        }
                        None
                    };
                    let legacy_token = match authorization_token {
                        Some(token) => Some(expand_env_var(token, trusted).await),
                        None => None,
                    };
                    // Remote declarations in a repository or plugin may still
                    // be connected explicitly by the user, but never create a
                    // network request merely because the workspace was opened.
                    let auto_connect =
                        trusted && options.auto_connect.unwrap_or(true) && options.enabled;

                    McpServerConfig::Remote {
                        url: expand_env_var(url, trusted).await,
                        transport: transport.clone(),
                        authorization_token: env_token.or(legacy_token),
                        oauth: oauth.clone(),
                        headers: resolved_headers,
                        options: resolve_options(options, source, auto_connect, authority),
                    }
                }
            };
            result.insert(name.clone(), config);
        }
        result
    }

    /// Get enabled, auto-connect-eligible remote server descriptors.
    ///
    /// The shape is connector-ready, but current provider request paths do not
    /// consume it; MCP execution remains routed through `McpManager`.
    pub async fn remote_servers_for_api(&self) -> Vec<RemoteMcpServer> {
        let resolved = self.servers().await;
        let mut result: Vec<_> = resolved
            .into_iter()
            .filter_map(|(name, server)| match server {
                McpServerConfig::Remote {
                    url,
                    authorization_token,
                    options,
                    ..
                } if options.enabled && options.auto_connect => Some(RemoteMcpServer {
                    server_type: "url".to_string(),
                    url,
                    name,
                    authorization_token,
                }),
                _ => None,
            })
            .collect();
        result.sort_by(|left, right| left.name.cmp(&right.name));
        result
    }
}

fn resolve_options(
    raw: &McpServerOptionsRaw,
    source: McpConfigSource,
    auto_connect: bool,
    authority: McpConnectionAuthority,
) -> McpServerOptions {
    McpServerOptions {
        enabled: raw.enabled,
        required: raw.required,
        auto_connect,
        startup_timeout_ms: raw.startup_timeout_ms.clamp(1, MAX_MCP_STARTUP_TIMEOUT_MS),
        tool_timeout_ms: raw.tool_timeout_ms.clamp(1, MAX_MCP_TOOL_TIMEOUT_MS),
        tools: raw.tools.clone(),
        source,
        authority,
    }
}

fn ensure_server_limit(config: &McpConfig, source: &Path) -> Result<()> {
    if config.mcp_servers.len() > MAX_MCP_SERVERS {
        anyhow::bail!(
            "merged MCP configuration exceeds the limit of {} servers after {}",
            MAX_MCP_SERVERS,
            source.display()
        );
    }
    Ok(())
}

async fn read_optional_config(path: &Path) -> Result<Option<McpConfig>> {
    let metadata = match tokio::fs::metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!(path = %path.display(), "No MCP config found");
            return Ok(None);
        }
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to inspect {}", path.display()));
        }
    };
    if !metadata.is_file() {
        anyhow::bail!("MCP config is not a file: {}", path.display());
    }
    if metadata.len() > MAX_MCP_CONFIG_BYTES as u64 {
        anyhow::bail!(
            "MCP config exceeds the {} byte limit: {}",
            MAX_MCP_CONFIG_BYTES,
            path.display()
        );
    }

    let file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("Failed to open {}", path.display()))?;
    let mut content = Vec::with_capacity(metadata.len() as usize);
    file.take((MAX_MCP_CONFIG_BYTES + 1) as u64)
        .read_to_end(&mut content)
        .await
        .with_context(|| format!("Failed to read {}", path.display()))?;
    if content.len() > MAX_MCP_CONFIG_BYTES {
        anyhow::bail!(
            "MCP config exceeds the {} byte limit: {}",
            MAX_MCP_CONFIG_BYTES,
            path.display()
        );
    }
    let config: McpConfig = serde_json::from_slice(&content)
        .with_context(|| format!("Failed to parse {}", path.display()))?;
    if config.mcp_servers.len() > MAX_MCP_SERVERS {
        anyhow::bail!(
            "MCP config exceeds the limit of {} servers: {}",
            MAX_MCP_SERVERS,
            path.display()
        );
    }
    Ok(Some(config))
}
