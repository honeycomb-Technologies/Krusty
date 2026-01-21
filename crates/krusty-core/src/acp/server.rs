//! ACP Server - Main entry point for ACP mode
//!
//! Handles the stdio transport and message routing for ACP protocol.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use agent_client_protocol::{AgentSideConnection, Client};
use anyhow::Result;
use tokio::io::{stdin, stdout};
use tokio::sync::mpsc;
use tokio::task::LocalSet;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{error, info, warn};

use super::agent::KrustyAgent;
use crate::ai::providers::ProviderId;
use crate::storage::credentials::{ActiveProviderStore, CredentialStore};
use crate::tools::ToolRegistry;

/// ACP Server configuration
#[derive(Debug, Clone, Default)]
pub struct AcpServerConfig {
    /// Working directory override
    pub working_dir: Option<std::path::PathBuf>,
}

/// ACP Server that runs Krusty as an ACP-compatible agent
pub struct AcpServer {
    agent: Arc<KrustyAgent>,
    #[allow(dead_code)]
    config: AcpServerConfig,
}

impl AcpServer {
    /// Create a new ACP server with default configuration
    pub fn new() -> Result<Self> {
        Ok(Self {
            agent: Arc::new(KrustyAgent::new()),
            config: AcpServerConfig::default(),
        })
    }

    /// Create with custom tool registry
    pub fn with_tools(tools: Arc<ToolRegistry>) -> Result<Self> {
        Ok(Self {
            agent: Arc::new(KrustyAgent::with_tools(tools)),
            config: AcpServerConfig::default(),
        })
    }

    /// Create with configuration
    pub fn with_config(config: AcpServerConfig) -> Result<Self> {
        Ok(Self {
            agent: Arc::new(KrustyAgent::new()),
            config,
        })
    }

    /// Run the ACP server (blocks until connection closes)
    ///
    /// This method takes over stdin/stdout for ACP communication.
    /// All logging should go to stderr.
    pub async fn run(self) -> Result<()> {
        info!("Starting Krusty ACP server");

        // Auto-initialize AI client from environment variables
        if let Some(config) = detect_api_key_from_env() {
            info!(
                "Auto-initializing AI client: provider={:?}, model={:?}",
                config.provider,
                config.model.as_deref().unwrap_or("default")
            );
            self.agent.init_ai_client_with_model(config.api_key, config.provider, config.model).await;
        } else {
            warn!(
                "No API key found in environment. Set one of:\n\
                 - KRUSTY_PROVIDER + KRUSTY_API_KEY (+ optional KRUSTY_MODEL)\n\
                 - ANTHROPIC_API_KEY\n\
                 - OPENROUTER_API_KEY\n\
                 - OPENCODEZEN_API_KEY\n\
                 - ZAI_API_KEY\n\
                 - MINIMAX_API_KEY\n\
                 - KIMI_API_KEY"
            );
        }

        // Create the agent-side connection
        // Note: ACP connections are not Send, so we need LocalSet
        let local = LocalSet::new();

        local
            .run_until(async move {
                // Create notification channel
                let (tx, mut rx) = mpsc::unbounded_channel();

                // Give the sender to the agent
                self.agent.set_notification_channel(tx).await;

                // Get stdin/stdout for transport, wrapped for futures compatibility
                let stdin = stdin().compat();
                let stdout = stdout().compat_write();

                // Spawn function for the connection
                let spawn_fn = |fut: Pin<Box<dyn Future<Output = ()>>>| {
                    tokio::task::spawn_local(fut);
                };

                // Create connection with our agent
                let (connection, io_task) = AgentSideConnection::new(
                    self.agent,
                    stdout,
                    stdin,
                    spawn_fn,
                );

                info!("ACP connection established, waiting for requests...");

                // Spawn task to forward notifications to the connection
                tokio::task::spawn_local(async move {
                    while let Some(notification) = rx.recv().await {
                        if let Err(e) = connection.session_notification(notification).await {
                            warn!("Failed to forward notification: {}", e);
                        }
                    }
                });

                // Run the IO task
                if let Err(e) = io_task.await {
                    error!("ACP connection error: {}", e);
                    return Err(anyhow::anyhow!("ACP connection error: {}", e));
                }

                info!("ACP connection closed");
                Ok(())
            })
            .await
    }

    /// Get a reference to the agent
    pub fn agent(&self) -> &KrustyAgent {
        &self.agent
    }
}

impl Default for AcpServer {
    fn default() -> Self {
        Self::new().expect("Failed to create default ACP server")
    }
}

/// Configuration detected from environment variables
#[derive(Debug, Clone)]
pub struct AcpEnvConfig {
    pub api_key: String,
    pub provider: ProviderId,
    pub model: Option<String>,
}

/// Detect API key and provider configuration
///
/// Checks in order:
/// 1. Environment variables (KRUSTY_PROVIDER + KRUSTY_API_KEY)
/// 2. Provider-specific env vars (ANTHROPIC_API_KEY, OPENROUTER_API_KEY, etc.)
/// 3. Krusty's stored credentials (~/.krusty/tokens/credentials.json)
///
/// Environment variable options:
/// - KRUSTY_PROVIDER: anthropic, openrouter, opencodezen, zai, minimax, kimi
/// - KRUSTY_MODEL: Override the default model for the provider
/// - KRUSTY_API_KEY: Generic API key (used with KRUSTY_PROVIDER)
fn detect_api_key_from_env() -> Option<AcpEnvConfig> {
    let model = std::env::var("KRUSTY_MODEL").ok().filter(|s| !s.is_empty());

    // Check for explicit provider configuration first
    if let Ok(provider_str) = std::env::var("KRUSTY_PROVIDER") {
        let provider = match provider_str.to_lowercase().as_str() {
            "anthropic" => Some(ProviderId::Anthropic),
            "openrouter" => Some(ProviderId::OpenRouter),
            "opencodezen" | "opencode" => Some(ProviderId::OpenCodeZen),
            "zai" | "z.ai" => Some(ProviderId::ZAi),
            "minimax" => Some(ProviderId::MiniMax),
            "kimi" => Some(ProviderId::Kimi),
            _ => None,
        };

        if let Some(provider) = provider {
            // Look for KRUSTY_API_KEY or provider-specific key
            let api_key = std::env::var("KRUSTY_API_KEY")
                .ok()
                .filter(|s| !s.is_empty())
                .or_else(|| get_provider_api_key(provider));

            if let Some(api_key) = api_key {
                return Some(AcpEnvConfig { api_key, provider, model });
            }
        }
    }

    // Fall back to checking provider-specific environment variables
    let providers_and_vars = [
        (ProviderId::Anthropic, "ANTHROPIC_API_KEY"),
        (ProviderId::OpenRouter, "OPENROUTER_API_KEY"),
        (ProviderId::OpenCodeZen, "OPENCODEZEN_API_KEY"),
        (ProviderId::ZAi, "ZAI_API_KEY"),
        (ProviderId::MiniMax, "MINIMAX_API_KEY"),
        (ProviderId::Kimi, "KIMI_API_KEY"),
        // OpenAI key maps to OpenRouter (which supports OpenAI models)
        (ProviderId::OpenRouter, "OPENAI_API_KEY"),
    ];

    for (provider, env_var) in providers_and_vars {
        if let Ok(key) = std::env::var(env_var) {
            if !key.is_empty() {
                return Some(AcpEnvConfig { api_key: key, provider, model });
            }
        }
    }

    // Fall back to Krusty's stored credentials
    if let Some(config) = detect_from_credential_store(model) {
        return Some(config);
    }

    None
}

/// Detect API key from Krusty's credential store
fn detect_from_credential_store(model: Option<String>) -> Option<AcpEnvConfig> {
    // Load stored credentials
    let store = match CredentialStore::load() {
        Ok(store) => store,
        Err(e) => {
            warn!("Failed to load credential store: {}", e);
            return None;
        }
    };

    // Get the active provider, or find the first configured one
    let active_provider = ActiveProviderStore::load();

    // Try active provider first
    if let Some(api_key) = store.get(&active_provider) {
        info!("Using active provider {:?} from credential store", active_provider);
        return Some(AcpEnvConfig {
            api_key: api_key.clone(),
            provider: active_provider,
            model,
        });
    }

    // Fall back to first configured provider
    let configured = store.configured_providers();
    if let Some(provider) = configured.first() {
        if let Some(api_key) = store.get(provider) {
            info!("Using first configured provider {:?} from credential store", provider);
            return Some(AcpEnvConfig {
                api_key: api_key.clone(),
                provider: *provider,
                model,
            });
        }
    }

    None
}

/// Get API key for a specific provider from environment
fn get_provider_api_key(provider: ProviderId) -> Option<String> {
    let env_var = match provider {
        ProviderId::Anthropic => "ANTHROPIC_API_KEY",
        ProviderId::OpenRouter => "OPENROUTER_API_KEY",
        ProviderId::OpenCodeZen => "OPENCODEZEN_API_KEY",
        ProviderId::ZAi => "ZAI_API_KEY",
        ProviderId::MiniMax => "MINIMAX_API_KEY",
        ProviderId::Kimi => "KIMI_API_KEY",
    };
    std::env::var(env_var).ok().filter(|s| !s.is_empty())
}

/// Check if we should run in ACP mode
///
/// Returns true if stdin is not a TTY (likely being spawned by an editor)
/// and the `--acp` flag is present.
#[allow(dead_code)]
pub fn should_run_acp_mode(force_acp: bool) -> bool {
    if force_acp {
        return true;
    }

    // Auto-detect: if stdin is not a TTY, we might be in ACP mode
    // But only if explicitly requested or detected
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_creation() {
        let server = AcpServer::new().unwrap();
        assert_eq!(server.agent().sessions().session_count(), 0);
    }

    #[test]
    fn test_acp_mode_detection() {
        assert!(should_run_acp_mode(true));
        assert!(!should_run_acp_mode(false));
    }
}
