//! OAuth 2.1/PKCE support for remote MCP servers.
//!
//! rmcp owns protocol discovery, dynamic client registration, PKCE, CSRF,
//! exchange, and refresh semantics. This module supplies Mitsuro's durable
//! credential boundary and a small coordinator for browser-facing flows.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use rmcp::transport::auth::{
    AuthError, AuthorizationManager, AuthorizationMetadata, AuthorizationSession,
    CredentialStore as RmcpCredentialStore, OAuthClientConfig, StateStore,
    StoredAuthorizationState, StoredCredentials,
};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use tokio::sync::{Mutex, RwLock};
use url::Host;

use crate::storage::CredentialStore;

use super::config::McpOAuthConfig;

const FLOW_TTL_SECS: u64 = 10 * 60;
const CREDENTIAL_KEY_PREFIX: &str = "mcp.oauth.v1";

#[derive(Debug, Clone, Copy)]
struct OAuthDeadlines {
    discovery: Duration,
    registration: Duration,
    exchange: Duration,
    restore: Duration,
    refresh: Duration,
    http_request: Duration,
}

impl Default for OAuthDeadlines {
    fn default() -> Self {
        Self {
            discovery: Duration::from_secs(20),
            registration: Duration::from_secs(20),
            exchange: Duration::from_secs(30),
            restore: Duration::from_secs(20),
            refresh: Duration::from_secs(30),
            // This remains attached to the manager after startup so refreshes
            // initiated later by rmcp's live HTTP transport are also bounded.
            http_request: Duration::from_secs(30),
        }
    }
}

/// Browser authorization state for one configured MCP server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum McpOAuthState {
    Disabled,
    AuthorizationRequired,
    Pending,
    Authenticated,
}

/// Secret-free OAuth status suitable for CLI, web, and mobile surfaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpOAuthStatus {
    pub server: String,
    pub state: McpOAuthState,
    pub configured_scopes: Vec<String>,
    pub granted_scopes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flow_expires_at: Option<u64>,
}

impl McpOAuthStatus {
    pub(crate) fn disabled(server: &str) -> Self {
        Self {
            server: server.to_string(),
            state: McpOAuthState::Disabled,
            configured_scopes: Vec::new(),
            granted_scopes: Vec::new(),
            flow_expires_at: None,
        }
    }
}

/// Result returned when a PKCE browser flow is created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpOAuthStart {
    pub server: String,
    pub authorization_url: String,
    pub expires_at: u64,
    pub scopes: Vec<String>,
}

struct PendingFlow {
    manager: AuthorizationManager,
    resource_url: String,
    expires_at: u64,
    scopes: Vec<String>,
    csrf_state: String,
    state_store: RetainedStateStore,
}

/// rmcp normally deletes PKCE state before it performs the token request. Keep
/// it retryable until Mitsuro observes a successful exchange, while the outer
/// coordinator serializes callbacks and enforces one-time terminal handling.
#[derive(Debug, Clone, Default)]
struct RetainedStateStore {
    states: Arc<RwLock<HashMap<String, StoredAuthorizationState>>>,
}

impl RetainedStateStore {
    async fn consume(&self, csrf_state: &str) {
        self.states.write().await.remove(csrf_state);
    }

    async fn clear(&self) {
        self.states.write().await.clear();
    }
}

#[async_trait]
impl StateStore for RetainedStateStore {
    async fn save(
        &self,
        csrf_token: &str,
        state: StoredAuthorizationState,
    ) -> std::result::Result<(), AuthError> {
        self.states
            .write()
            .await
            .insert(csrf_token.to_string(), state);
        Ok(())
    }

    async fn load(
        &self,
        csrf_token: &str,
    ) -> std::result::Result<Option<StoredAuthorizationState>, AuthError> {
        Ok(self.states.read().await.get(csrf_token).cloned())
    }

    async fn delete(&self, _csrf_token: &str) -> std::result::Result<(), AuthError> {
        // Deliberately retained. McpOAuthCoordinator consumes it only after a
        // validated callback reaches a successful terminal state.
        Ok(())
    }
}

pub(crate) struct McpOAuthCoordinator {
    pending: RwLock<HashMap<String, Arc<PendingFlow>>>,
    lifecycle: Mutex<()>,
    credential_path: PathBuf,
    deadlines: OAuthDeadlines,
}

impl McpOAuthCoordinator {
    pub(crate) fn new() -> Self {
        Self::with_credential_path(CredentialStore::path())
    }

    fn with_credential_path(credential_path: PathBuf) -> Self {
        Self::with_credential_path_and_deadlines(credential_path, OAuthDeadlines::default())
    }

    fn with_credential_path_and_deadlines(
        credential_path: PathBuf,
        deadlines: OAuthDeadlines,
    ) -> Self {
        Self {
            pending: RwLock::new(HashMap::new()),
            lifecycle: Mutex::new(()),
            credential_path,
            deadlines,
        }
    }

    pub(crate) async fn start(
        &self,
        server: &str,
        resource_url: &str,
        config: &McpOAuthConfig,
        redirect_uri: &str,
    ) -> Result<McpOAuthStart> {
        validate_secure_url(resource_url, "MCP resource URL")?;
        validate_redirect_uri(redirect_uri)?;
        validate_oauth_config(config)?;
        if let Some(client_metadata_url) = config.client_metadata_url.as_deref() {
            validate_secure_url(client_metadata_url, "MCP OAuth client metadata URL")?;
        }
        if config.client_id.is_some() && config.client_metadata_url.is_some() {
            bail!("MCP OAuth config cannot set both clientId and clientMetadataUrl");
        }

        let _lifecycle = self.lifecycle.lock().await;
        self.prune_expired_unlocked().await;
        let store = self.store(resource_url)?;
        let mut manager = self.new_authorization_manager(resource_url).await?;
        manager.set_credential_store(store);
        let state_store = RetainedStateStore::default();
        manager.set_state_store(state_store.clone());

        let metadata = run_auth_operation(
            self.deadlines.discovery,
            "MCP OAuth metadata discovery",
            manager.discover_metadata(),
        )
        .await
        .context("MCP server did not expose valid OAuth metadata")?;
        validate_authorization_metadata(&metadata)?;
        manager.set_metadata(metadata);

        let scopes = if config.scopes.is_empty() {
            manager.select_scopes(None, &[])
        } else {
            config.scopes.clone()
        };
        let scope_refs: Vec<&str> = scopes.iter().map(String::as_str).collect();

        let (manager, authorization_url) = if let Some(client_id) = &config.client_id {
            let client =
                OAuthClientConfig::new(client_id, redirect_uri).with_scopes(scopes.clone());
            manager
                .configure_client(client)
                .context("failed to configure MCP OAuth clientId")?;
            let authorization_url = run_auth_operation(
                self.deadlines.registration,
                "MCP OAuth authorization URL generation",
                manager.get_authorization_url(&scope_refs),
            )
            .await
            .context("failed to create MCP OAuth authorization URL")?;
            (manager, authorization_url)
        } else {
            let session = run_auth_operation(
                self.deadlines.registration,
                "MCP OAuth dynamic client registration",
                AuthorizationSession::new(
                    manager,
                    &scope_refs,
                    redirect_uri,
                    Some(config.client_name()),
                    config.client_metadata_url.as_deref(),
                ),
            )
            .await
            .context("failed to register MCP OAuth client")?;
            (session.auth_manager, session.auth_url)
        };
        validate_secure_url(&authorization_url, "MCP OAuth authorization URL")?;
        let csrf_state = authorization_state(&authorization_url)?;

        let expires_at = unix_timestamp().saturating_add(FLOW_TTL_SECS);
        let replaced = self.pending.write().await.insert(
            server.to_string(),
            Arc::new(PendingFlow {
                manager,
                resource_url: resource_url.to_string(),
                expires_at,
                scopes: scopes.clone(),
                csrf_state,
                state_store,
            }),
        );
        if let Some(replaced) = replaced {
            replaced.state_store.clear().await;
        }

        Ok(McpOAuthStart {
            server: server.to_string(),
            authorization_url,
            expires_at,
            scopes,
        })
    }

    pub(crate) async fn complete(&self, server: &str, code: &str, state: &str) -> Result<()> {
        if code.is_empty() || code.len() > 16 * 1024 {
            bail!("invalid MCP OAuth authorization code");
        }
        validate_callback_state(state)?;

        let _lifecycle = self.lifecycle.lock().await;
        let pending = self
            .pending
            .read()
            .await
            .get(server)
            .cloned()
            .with_context(|| format!("no pending OAuth authorization for MCP server '{server}'"))?;
        if !constant_time_eq(state.as_bytes(), pending.csrf_state.as_bytes()) {
            bail!("invalid MCP OAuth state for server '{server}'");
        }
        if pending.expires_at <= unix_timestamp() {
            pending.state_store.consume(&pending.csrf_state).await;
            self.pending.write().await.remove(server);
            bail!("OAuth authorization for MCP server '{server}' expired; start it again");
        }

        run_auth_operation(
            self.deadlines.exchange,
            "MCP OAuth token exchange",
            pending.manager.exchange_code_for_token(code, state),
        )
        .await
        .context("MCP OAuth callback validation or token exchange failed")?;
        pending.state_store.consume(&pending.csrf_state).await;
        self.pending.write().await.remove(server);
        Ok(())
    }

    pub(crate) async fn cancel(&self, server: &str) {
        let _lifecycle = self.lifecycle.lock().await;
        if let Some(flow) = self.pending.write().await.remove(server) {
            flow.state_store.clear().await;
        }
    }

    /// Finish a provider-declared OAuth error only when it belongs to the
    /// pending browser flow. Untrusted callbacks with a missing or mismatched
    /// state must not cancel the legitimate authorization attempt.
    pub(crate) async fn cancel_with_state(&self, server: &str, state: &str) -> Result<()> {
        validate_callback_state(state)?;
        let _lifecycle = self.lifecycle.lock().await;
        let pending = self
            .pending
            .read()
            .await
            .get(server)
            .cloned()
            .with_context(|| format!("no pending OAuth authorization for MCP server '{server}'"))?;
        if !constant_time_eq(state.as_bytes(), pending.csrf_state.as_bytes()) {
            bail!("invalid MCP OAuth state for server '{server}'");
        }

        pending.state_store.consume(&pending.csrf_state).await;
        self.pending.write().await.remove(server);
        if pending.expires_at <= unix_timestamp() {
            bail!("OAuth authorization for MCP server '{server}' expired; start it again");
        }
        Ok(())
    }

    pub(crate) async fn logout(&self, server: &str, resource_url: &str) -> Result<()> {
        self.cancel(server).await;
        self.store(resource_url)?
            .clear()
            .await
            .context("failed to clear MCP OAuth credentials")
    }

    pub(crate) async fn status(
        &self,
        server: &str,
        resource_url: &str,
        config: &McpOAuthConfig,
    ) -> Result<McpOAuthStatus> {
        self.prune_expired().await;
        let pending = self.pending.read().await;
        if let Some(flow) = pending.get(server) {
            return Ok(McpOAuthStatus {
                server: server.to_string(),
                state: McpOAuthState::Pending,
                configured_scopes: flow.scopes.clone(),
                granted_scopes: Vec::new(),
                flow_expires_at: Some(flow.expires_at),
            });
        }
        drop(pending);

        let stored = self
            .store(resource_url)?
            .load()
            .await
            .context("failed to load MCP OAuth credentials")?;
        let (state, granted_scopes) = match stored {
            Some(credentials) if credentials.token_response.is_some() => {
                (McpOAuthState::Authenticated, credentials.granted_scopes)
            }
            _ => (McpOAuthState::AuthorizationRequired, Vec::new()),
        };
        Ok(McpOAuthStatus {
            server: server.to_string(),
            state,
            configured_scopes: config.scopes.clone(),
            granted_scopes,
            flow_expires_at: None,
        })
    }

    /// Build a refresh-capable rmcp authorization manager for the live HTTP
    /// transport. `get_access_token` is called once here for an actionable
    /// startup diagnostic and again by rmcp for every HTTP request.
    pub(crate) async fn authorization_manager(
        &self,
        server: &str,
        resource_url: &str,
    ) -> Result<AuthorizationManager> {
        validate_secure_url(resource_url, "MCP resource URL")?;
        let mut manager = self.new_authorization_manager(resource_url).await?;
        manager.set_credential_store(self.store(resource_url)?);
        // Do not let rmcp discover and immediately consume endpoints from a
        // stored credential internally. Validate every endpoint first, then
        // install the trusted metadata snapshot before restore/refresh.
        let metadata = run_auth_operation(
            self.deadlines.discovery,
            "MCP OAuth metadata discovery",
            manager.discover_metadata(),
        )
        .await
        .context("MCP server did not expose valid OAuth metadata")?;
        validate_authorization_metadata(&metadata)?;
        manager.set_metadata(metadata);
        let restored = run_auth_operation(
            self.deadlines.restore,
            "MCP OAuth credential restore",
            manager.initialize_from_store(),
        )
        .await
        .context("failed to restore MCP OAuth credentials")?;
        if !restored {
            bail!(
                "OAuth authorization required for MCP server '{server}'; start the browser authorization flow"
            );
        }
        run_auth_operation(
            self.deadlines.refresh,
            "MCP OAuth access-token refresh",
            manager.get_access_token(),
        )
        .await
        .with_context(|| format!("OAuth authorization required for MCP server '{server}'"))?;
        Ok(manager)
    }

    pub(crate) async fn retain_servers(&self, resources: &HashMap<String, String>) -> Result<()> {
        let _lifecycle = self.lifecycle.lock().await;
        let removed = {
            let mut pending = self.pending.write().await;
            let removed_names = pending
                .iter()
                .filter(|(name, flow)| {
                    resources
                        .get(*name)
                        .is_none_or(|url| url != &flow.resource_url)
                })
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            removed_names
                .into_iter()
                .filter_map(|name| pending.remove(&name))
                .collect::<Vec<_>>()
        };
        for flow in removed {
            flow.state_store.clear().await;
        }
        self.retain_credentials_for_resources(resources).await
    }

    /// Invalidates in-flight browser authorizations without changing durable
    /// credentials. A malformed configuration must fail closed for the active
    /// runtime snapshot, but it is not evidence that the user revoked every
    /// previously authorized MCP resource.
    pub(crate) async fn clear_pending_flows(&self) {
        let _lifecycle = self.lifecycle.lock().await;
        let removed = self
            .pending
            .write()
            .await
            .drain()
            .map(|(_, flow)| flow)
            .collect::<Vec<_>>();
        for flow in removed {
            flow.state_store.clear().await;
        }
    }

    async fn prune_expired(&self) {
        let _lifecycle = self.lifecycle.lock().await;
        self.prune_expired_unlocked().await;
    }

    async fn prune_expired_unlocked(&self) {
        let now = unix_timestamp();
        let expired = {
            let mut pending = self.pending.write().await;
            let names = pending
                .iter()
                .filter(|(_, flow)| flow.expires_at <= now)
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            names
                .into_iter()
                .filter_map(|name| pending.remove(&name))
                .collect::<Vec<_>>()
        };
        for flow in expired {
            flow.state_store.clear().await;
        }
    }

    fn store(&self, resource_url: &str) -> Result<RepositoryOAuthCredentialStore> {
        RepositoryOAuthCredentialStore::new(resource_url, self.credential_path.clone())
    }

    async fn new_authorization_manager(&self, resource_url: &str) -> Result<AuthorizationManager> {
        ensure_rustls_crypto_provider();
        let mut manager = AuthorizationManager::new(resource_url)
            .await
            .context("failed to initialize MCP OAuth")?;
        let client = reqwest_rmcp::Client::builder()
            .timeout(self.deadlines.http_request)
            .redirect(reqwest_rmcp::redirect::Policy::none())
            .build()
            .context("failed to build deadline-bound MCP OAuth HTTP client")?;
        manager
            .with_client(client)
            .context("failed to configure MCP OAuth HTTP client")?;
        Ok(manager)
    }

    async fn retain_credentials_for_resources(
        &self,
        resources: &HashMap<String, String>,
    ) -> Result<()> {
        let allowed = resources
            .values()
            .map(|resource_url| self.store(resource_url).map(|store| store.key))
            .collect::<Result<HashSet<_>>>()?;
        let path = self.credential_path.clone();
        let prefix = format!("{CREDENTIAL_KEY_PREFIX}.");
        let _credential_guard = CREDENTIAL_IO_LOCK.lock().await;
        tokio::task::spawn_blocking(move || {
            let mut store = CredentialStore::load_from_path(&path)?;
            let removed = store
                .scoped_secret_keys_with_prefix(&prefix)
                .into_iter()
                .filter(|key| !allowed.contains(key))
                .collect::<Vec<_>>();
            if removed.is_empty() {
                return Ok(());
            }
            for key in removed {
                store.remove_scoped_secret(&key);
            }
            store.save_to_path(&path)
        })
        .await
        .context("MCP OAuth credential cleanup task failed")?
        .context("failed to clear credentials for removed MCP OAuth resources")
    }
}

/// reqwest 0.13's `rustls-no-provider` feature requires the application to
/// choose a process-wide provider before constructing a client. Preserve an
/// earlier application choice; otherwise select the same ring provider used by
/// Mitsuro's reqwest 0.12 and WebSocket clients. Losing an initialization race
/// is harmless because the winning provider is then already installed.
fn ensure_rustls_crypto_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

/// rmcp storage adapter backed by Mitsuro's existing atomic, owner-only
/// credential file. The resource URL is SHA-256 namespaced so a token can
/// never be reused for a different MCP audience due to a server-name clash.
#[derive(Debug, Clone)]
struct RepositoryOAuthCredentialStore {
    key: String,
    path: PathBuf,
}

static CREDENTIAL_IO_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

impl RepositoryOAuthCredentialStore {
    fn new(resource_url: &str, path: PathBuf) -> Result<Self> {
        let parsed = reqwest::Url::parse(resource_url).context("invalid MCP resource URL")?;
        let digest = Sha256::digest(parsed.as_str().as_bytes());
        Ok(Self {
            key: format!("{CREDENTIAL_KEY_PREFIX}.{:x}", digest),
            path,
        })
    }

    fn load_sync(path: &Path, key: &str) -> Result<Option<StoredCredentials>> {
        let store = CredentialStore::load_from_path(path)?;
        store
            .get_scoped_secret(key)
            .map(|value| serde_json::from_str(value).context("invalid stored MCP OAuth credential"))
            .transpose()
    }

    fn save_sync(path: &Path, key: &str, credentials: &StoredCredentials) -> Result<()> {
        let mut store = CredentialStore::load_from_path(path)?;
        let serialized = serde_json::to_string(credentials)?;
        store.set_scoped_secret(key.to_string(), serialized);
        store.save_to_path(path)
    }

    fn clear_sync(path: &Path, key: &str) -> Result<()> {
        let mut store = CredentialStore::load_from_path(path)?;
        store.remove_scoped_secret(key);
        store.save_to_path(path)
    }
}

#[async_trait]
impl RmcpCredentialStore for RepositoryOAuthCredentialStore {
    async fn load(&self) -> std::result::Result<Option<StoredCredentials>, AuthError> {
        let _guard = CREDENTIAL_IO_LOCK.lock().await;
        let path = self.path.clone();
        let key = self.key.clone();
        tokio::task::spawn_blocking(move || Self::load_sync(&path, &key))
            .await
            .map_err(|error| credential_error(error.to_string()))?
            .map_err(|error| credential_error(error.to_string()))
    }

    async fn save(&self, credentials: StoredCredentials) -> std::result::Result<(), AuthError> {
        let _guard = CREDENTIAL_IO_LOCK.lock().await;
        let path = self.path.clone();
        let key = self.key.clone();
        tokio::task::spawn_blocking(move || Self::save_sync(&path, &key, &credentials))
            .await
            .map_err(|error| credential_error(error.to_string()))?
            .map_err(|error| credential_error(error.to_string()))
    }

    async fn clear(&self) -> std::result::Result<(), AuthError> {
        let _guard = CREDENTIAL_IO_LOCK.lock().await;
        let path = self.path.clone();
        let key = self.key.clone();
        tokio::task::spawn_blocking(move || Self::clear_sync(&path, &key))
            .await
            .map_err(|error| credential_error(error.to_string()))?
            .map_err(|error| credential_error(error.to_string()))
    }
}

async fn run_auth_operation<T, F>(deadline: Duration, operation: &str, future: F) -> Result<T>
where
    F: Future<Output = std::result::Result<T, AuthError>>,
{
    match tokio::time::timeout(deadline, future).await {
        Ok(result) => result.with_context(|| format!("{operation} failed")),
        Err(_) => bail!(
            "{operation} timed out after {} milliseconds",
            deadline.as_millis()
        ),
    }
}

fn authorization_state(authorization_url: &str) -> Result<String> {
    let parsed =
        reqwest::Url::parse(authorization_url).context("invalid MCP OAuth authorization URL")?;
    let state = parsed
        .query_pairs()
        .find_map(|(key, value)| (key == "state").then(|| value.into_owned()))
        .context("MCP OAuth authorization URL is missing state")?;
    validate_callback_state(&state)?;
    Ok(state)
}

fn validate_callback_state(state: &str) -> Result<()> {
    if state.is_empty() || state.len() > 4 * 1024 || state.chars().any(char::is_control) {
        bail!("invalid MCP OAuth state");
    }
    Ok(())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let max_len = left.len().max(right.len());
    let mut difference = left.len() ^ right.len();
    for index in 0..max_len {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        difference |= usize::from(left_byte ^ right_byte);
    }
    difference == 0
}

fn credential_error(message: String) -> AuthError {
    AuthError::InternalError(format!("Mitsuro credential store failure: {message}"))
}

fn validate_redirect_uri(value: &str) -> Result<()> {
    validate_secure_url(value, "OAuth redirect URI")
}

fn validate_oauth_config(config: &McpOAuthConfig) -> Result<()> {
    if config.client_id.as_ref().is_some_and(|value| {
        value.trim().is_empty() || value.len() > 2048 || value.chars().any(char::is_control)
    }) {
        bail!("MCP OAuth clientId must contain 1 to 2048 non-control characters");
    }
    if config.client_name.as_ref().is_some_and(|value| {
        value.trim().is_empty() || value.len() > 128 || value.chars().any(char::is_control)
    }) {
        bail!("MCP OAuth clientName must contain 1 to 128 characters");
    }
    for (index, scope) in config.scopes.iter().enumerate() {
        let valid = !scope.is_empty()
            && scope.bytes().all(|byte| {
                byte == 0x21 || (0x23..=0x5b).contains(&byte) || (0x5d..=0x7e).contains(&byte)
            });
        if !valid {
            bail!("invalid MCP OAuth scope at index {index}");
        }
    }
    Ok(())
}

fn validate_authorization_metadata(metadata: &AuthorizationMetadata) -> Result<()> {
    validate_secure_url(
        &metadata.authorization_endpoint,
        "MCP OAuth authorization endpoint",
    )?;
    validate_secure_url(&metadata.token_endpoint, "MCP OAuth token endpoint")?;
    if let Some(registration_endpoint) = metadata.registration_endpoint.as_deref() {
        validate_secure_url(registration_endpoint, "MCP OAuth registration endpoint")?;
    }
    Ok(())
}

fn validate_secure_url(value: &str, label: &str) -> Result<()> {
    let parsed = reqwest::Url::parse(value).with_context(|| format!("invalid {label}"))?;
    let secure = parsed.scheme() == "https";
    let loopback_http = parsed.scheme() == "http" && is_loopback_host(&parsed);
    if !secure && !loopback_http {
        bail!("{label} must use HTTPS (HTTP is allowed only for localhost/loopback)");
    }
    if parsed.host_str().is_none() || !parsed.username().is_empty() || parsed.password().is_some() {
        bail!("{label} must be an absolute URL without embedded credentials");
    }
    if parsed.fragment().is_some() {
        bail!("{label} must not contain a URL fragment");
    }
    Ok(())
}

fn is_loopback_host(url: &reqwest::Url) -> bool {
    match url.host() {
        Some(Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    }
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::transport::auth::OAuthTokenResponse;
    use std::collections::HashMap;
    use std::sync::mpsc;
    use std::time::Duration;

    fn stored_credentials() -> StoredCredentials {
        let token_response: OAuthTokenResponse = serde_json::from_value(serde_json::json!({
            "access_token": "secret-access-token",
            "token_type": "bearer",
            "expires_in": 3600,
            "refresh_token": "secret-refresh-token",
            "scope": "read write"
        }))
        .unwrap();
        serde_json::from_value(serde_json::json!({
            "client_id": "client-id",
            "token_response": token_response,
            "granted_scopes": ["read", "write"],
            "token_received_at": unix_timestamp()
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn credentials_round_trip_through_shared_secure_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("tokens").join("credentials.json");
        let store =
            RepositoryOAuthCredentialStore::new("https://mcp.example.com/mcp", path.clone())
                .unwrap();

        store.save(stored_credentials()).await.unwrap();
        let restored = store.load().await.unwrap().unwrap();
        assert_eq!(restored.client_id, "client-id");
        assert_eq!(restored.granted_scopes, ["read", "write"]);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        store.clear().await.unwrap();
        assert!(store.load().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn removed_resources_clear_only_their_namespaced_credentials() {
        let temp = tempfile::tempdir().unwrap();
        let coordinator =
            McpOAuthCoordinator::with_credential_path(temp.path().join("credentials.json"));
        let retained_url = "https://retained.example/mcp";
        let removed_url = "https://removed.example/mcp";
        coordinator
            .store(retained_url)
            .unwrap()
            .save(stored_credentials())
            .await
            .unwrap();
        coordinator
            .store(removed_url)
            .unwrap()
            .save(stored_credentials())
            .await
            .unwrap();

        coordinator
            .retain_servers(&HashMap::from([(
                "retained".to_string(),
                retained_url.to_string(),
            )]))
            .await
            .unwrap();

        assert!(coordinator
            .store(retained_url)
            .unwrap()
            .load()
            .await
            .unwrap()
            .is_some());
        assert!(coordinator
            .store(removed_url)
            .unwrap()
            .load()
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn clearing_pending_flows_preserves_durable_credentials() {
        let temp = tempfile::tempdir().unwrap();
        let coordinator =
            McpOAuthCoordinator::with_credential_path(temp.path().join("credentials.json"));
        let resource_url = "https://retained.example/mcp";
        coordinator
            .store(resource_url)
            .unwrap()
            .save(stored_credentials())
            .await
            .unwrap();

        coordinator.clear_pending_flows().await;

        assert!(coordinator
            .store(resource_url)
            .unwrap()
            .load()
            .await
            .unwrap()
            .is_some());
    }

    #[test]
    fn credentials_are_namespaced_by_resource_url() {
        let path = PathBuf::from("credentials.json");
        let first =
            RepositoryOAuthCredentialStore::new("https://one.example/mcp", path.clone()).unwrap();
        let second = RepositoryOAuthCredentialStore::new("https://two.example/mcp", path).unwrap();
        assert_ne!(first.key, second.key);
        assert!(!first.key.contains("one.example"));
    }

    #[test]
    fn oauth_urls_require_tls_except_for_loopback() {
        assert!(validate_secure_url("https://mcp.example/mcp", "test").is_ok());
        assert!(validate_secure_url("http://localhost:3000/callback", "test").is_ok());
        assert!(validate_secure_url("http://127.0.0.1:3000/callback", "test").is_ok());
        assert!(validate_secure_url("http://mcp.example/mcp", "test").is_err());
        assert!(validate_secure_url("https://user:pass@mcp.example/mcp", "test").is_err());
    }

    #[test]
    fn discovered_oauth_endpoints_are_validated_before_use() {
        let valid: AuthorizationMetadata = serde_json::from_value(serde_json::json!({
            "authorization_endpoint": "https://auth.example/authorize",
            "token_endpoint": "https://auth.example/token",
            "registration_endpoint": "https://auth.example/register"
        }))
        .unwrap();
        assert!(validate_authorization_metadata(&valid).is_ok());

        for (field, label) in [
            ("authorization_endpoint", "authorization endpoint"),
            ("token_endpoint", "token endpoint"),
            ("registration_endpoint", "registration endpoint"),
        ] {
            let mut value = serde_json::json!({
                "authorization_endpoint": "https://auth.example/authorize",
                "token_endpoint": "https://auth.example/token",
                "registration_endpoint": "https://auth.example/register"
            });
            value[field] = serde_json::json!(format!("http://attacker.example/{field}"));
            let metadata: AuthorizationMetadata = serde_json::from_value(value).unwrap();
            let error = validate_authorization_metadata(&metadata).unwrap_err();
            assert!(error.to_string().contains(label), "{error:#}");
            assert!(error.to_string().contains("must use HTTPS"), "{error:#}");
        }

        let loopback: AuthorizationMetadata = serde_json::from_value(serde_json::json!({
            "authorization_endpoint": "http://127.0.0.1:4000/authorize",
            "token_endpoint": "http://localhost:4000/token",
            "registration_endpoint": "http://[::1]:4000/register"
        }))
        .unwrap();
        assert!(validate_authorization_metadata(&loopback).is_ok());
    }

    #[test]
    fn oauth_config_rejects_ambiguous_scope_tokens() {
        let config = McpOAuthConfig {
            scopes: vec!["read write".to_string()],
            ..McpOAuthConfig::default()
        };
        assert!(validate_oauth_config(&config).is_err());
    }

    #[tokio::test]
    async fn oauth_rejects_insecure_client_metadata_url_before_discovery() {
        let coordinator = McpOAuthCoordinator::with_credential_path(PathBuf::from("unused"));
        let config = McpOAuthConfig {
            client_metadata_url: Some("http://metadata.example/client.json".to_string()),
            ..McpOAuthConfig::default()
        };
        let error = coordinator
            .start(
                "example",
                "https://mcp.example/mcp",
                &config,
                "http://127.0.0.1:39177/oauth/callback",
            )
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("client metadata URL must use HTTPS"));
    }

    #[tokio::test]
    async fn oauth_discovery_obeys_the_coordinator_deadline() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        let resource_url = format!("http://{address}/mcp");
        let server_thread = std::thread::spawn(move || {
            if let Some(request) = server.recv_timeout(Duration::from_secs(2)).unwrap() {
                std::thread::sleep(Duration::from_millis(250));
                let _ = request.respond(tiny_http::Response::empty(504));
            }
        });
        let short = Duration::from_millis(50);
        let coordinator = McpOAuthCoordinator::with_credential_path_and_deadlines(
            tempfile::tempdir().unwrap().path().join("credentials.json"),
            OAuthDeadlines {
                discovery: short,
                registration: short,
                exchange: short,
                restore: short,
                refresh: short,
                http_request: Duration::from_secs(1),
            },
        );
        let started = std::time::Instant::now();
        let error = coordinator
            .start(
                "slow",
                &resource_url,
                &McpOAuthConfig {
                    client_id: Some("public-client".to_string()),
                    ..McpOAuthConfig::default()
                },
                "http://127.0.0.1:39177/oauth/callback",
            )
            .await
            .unwrap_err();

        assert!(
            error
                .chain()
                .any(|cause| cause.to_string().contains("timed out")),
            "{error:#}"
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        server_thread.join().unwrap();
    }

    #[tokio::test]
    async fn status_never_exposes_tokens() {
        let temp = tempfile::tempdir().unwrap();
        let coordinator =
            McpOAuthCoordinator::with_credential_path(temp.path().join("credentials.json"));
        let store = coordinator.store("https://mcp.example/mcp").unwrap();
        store.save(stored_credentials()).await.unwrap();

        let status = coordinator
            .status(
                "example",
                "https://mcp.example/mcp",
                &McpOAuthConfig {
                    scopes: vec!["read".to_string()],
                    ..McpOAuthConfig::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(status.state, McpOAuthState::Authenticated);
        let json = serde_json::to_string(&status).unwrap();
        assert!(!json.contains("secret-access-token"));
        assert!(!json.contains("secret-refresh-token"));
    }

    #[tokio::test]
    async fn pkce_callback_persists_and_refreshes_credentials_end_to_end() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        let base_url = format!("http://{address}");
        let resource_url = format!("{base_url}/mcp");
        let metadata_base = base_url.clone();
        let (request_tx, request_rx) = mpsc::channel();
        let server_thread = std::thread::spawn(move || {
            let mut token_requests = 0;
            while token_requests < 3 {
                let Some(mut request) = server.recv_timeout(Duration::from_secs(10)).unwrap()
                else {
                    break;
                };
                let path = request.url().to_string();
                if path.contains("/.well-known/oauth-authorization-server") {
                    let body = serde_json::json!({
                        "authorization_endpoint": format!("{metadata_base}/authorize"),
                        "token_endpoint": format!("{metadata_base}/token"),
                        "response_types_supported": ["code"],
                        "code_challenge_methods_supported": ["S256"],
                        "scopes_supported": ["read", "offline_access"]
                    })
                    .to_string();
                    respond_json(request, 200, body);
                } else if path == "/token" {
                    let mut body = String::new();
                    request.as_reader().read_to_string(&mut body).unwrap();
                    request_tx.send(body).unwrap();
                    token_requests += 1;
                    if token_requests == 1 {
                        respond_json(
                            request,
                            503,
                            serde_json::json!({
                                "error": "temporarily_unavailable"
                            })
                            .to_string(),
                        );
                        continue;
                    }
                    let token = if token_requests == 2 {
                        serde_json::json!({
                            "access_token": "initial-access-token",
                            "token_type": "bearer",
                            "expires_in": 1,
                            "refresh_token": "initial-refresh-token",
                            "scope": "read"
                        })
                    } else {
                        serde_json::json!({
                            "access_token": "refreshed-access-token",
                            "token_type": "bearer",
                            "expires_in": 3600,
                            "refresh_token": "refreshed-refresh-token",
                            "scope": "read"
                        })
                    };
                    respond_json(request, 200, token.to_string());
                } else {
                    request.respond(tiny_http::Response::empty(404)).unwrap();
                }
            }
        });

        let temp = tempfile::tempdir().unwrap();
        let coordinator = McpOAuthCoordinator::with_credential_path(
            temp.path().join("tokens").join("credentials.json"),
        );
        let config = McpOAuthConfig {
            scopes: vec!["read".to_string()],
            client_id: Some("public-test-client".to_string()),
            ..McpOAuthConfig::default()
        };
        let start = coordinator
            .start(
                "fixture",
                &resource_url,
                &config,
                "http://127.0.0.1:39177/oauth/callback",
            )
            .await
            .unwrap();
        let authorization_url = reqwest::Url::parse(&start.authorization_url).unwrap();
        let authorization_params: HashMap<_, _> =
            authorization_url.query_pairs().into_owned().collect();
        assert_eq!(
            authorization_params
                .get("code_challenge_method")
                .map(String::as_str),
            Some("S256")
        );
        let csrf_state = authorization_params.get("state").unwrap().clone();

        let wrong_state_error = coordinator
            .complete("fixture", "authorization-code", "wrong-state")
            .await
            .unwrap_err();
        assert!(wrong_state_error
            .to_string()
            .contains("invalid MCP OAuth state"));
        assert_eq!(
            coordinator
                .status("fixture", &resource_url, &config)
                .await
                .unwrap()
                .state,
            McpOAuthState::Pending
        );
        coordinator
            .cancel_with_state("fixture", "wrong-state")
            .await
            .expect_err("provider error with wrong state must not cancel the flow");

        coordinator
            .complete("fixture", "retryable-authorization-code", &csrf_state)
            .await
            .expect_err("temporary token endpoint failure should be retryable");
        let failed_exchange_body = request_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(failed_exchange_body.contains("retryable-authorization-code"));
        assert_eq!(
            coordinator
                .status("fixture", &resource_url, &config)
                .await
                .unwrap()
                .state,
            McpOAuthState::Pending
        );

        coordinator
            .complete("fixture", "authorization-code", &csrf_state)
            .await
            .unwrap();
        let exchange_body = request_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let exchange_params: HashMap<_, _> = url::form_urlencoded::parse(exchange_body.as_bytes())
            .into_owned()
            .collect();
        assert_eq!(
            exchange_params.get("grant_type").map(String::as_str),
            Some("authorization_code")
        );
        assert!(exchange_params
            .get("code_verifier")
            .is_some_and(|value| !value.is_empty()));
        assert_eq!(exchange_params.get("resource"), Some(&resource_url));

        let manager = coordinator
            .authorization_manager("fixture", &resource_url)
            .await
            .unwrap();
        assert_eq!(
            manager.get_access_token().await.unwrap(),
            "refreshed-access-token"
        );
        let refresh_body = request_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let refresh_params: HashMap<_, _> = url::form_urlencoded::parse(refresh_body.as_bytes())
            .into_owned()
            .collect();
        assert_eq!(
            refresh_params.get("grant_type").map(String::as_str),
            Some("refresh_token")
        );
        assert_eq!(
            refresh_params.get("refresh_token").map(String::as_str),
            Some("initial-refresh-token")
        );

        server_thread.join().unwrap();
        let stored = coordinator
            .store(&resource_url)
            .unwrap()
            .load()
            .await
            .unwrap()
            .unwrap();
        let stored_json = serde_json::to_string(&stored).unwrap();
        assert!(stored_json.contains("refreshed-access-token"));
        assert!(!stored_json.contains("initial-access-token"));
    }

    fn respond_json(request: tiny_http::Request, status: u16, body: String) {
        let content_type =
            tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap();
        request
            .respond(
                tiny_http::Response::from_string(body)
                    .with_status_code(status)
                    .with_header(content_type),
            )
            .unwrap();
    }
}
