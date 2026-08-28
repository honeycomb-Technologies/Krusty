use std::net::{SocketAddr, TcpListener as StdTcpListener};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use axum::{extract::DefaultBodyLimit, middleware, routing::get, Router};
use tower_http::trace::TraceLayer;

use crate::{
    auth, build_app_state_with_hive_runtime, health, routes, serve_web_app, AppState,
    HiveRuntimeMode, ServerConfig, ServerHttpPolicy,
};

/// Explicit authority for a disposable, Hive-enabled acceptance server.
///
/// Every path and the bind address are required by construction. This server
/// never performs daemon discovery and never falls back to Mitsuro's default
/// database.
#[derive(Debug, Clone)]
pub struct HiveAcceptanceServerConfig {
    pub bind_address: SocketAddr,
    pub database_path: PathBuf,
    pub working_dir: PathBuf,
    pub hive_socket_path: PathBuf,
    pub hive_key_path: PathBuf,
}

impl HiveAcceptanceServerConfig {
    fn validate(&self) -> Result<()> {
        if !self.bind_address.ip().is_loopback() {
            bail!(
                "Hive acceptance server must bind to a loopback address, not {}",
                self.bind_address.ip()
            );
        }
        for (name, path) in [
            ("database_path", self.database_path.as_path()),
            ("working_dir", self.working_dir.as_path()),
            ("hive_socket_path", self.hive_socket_path.as_path()),
            ("hive_key_path", self.hive_key_path.as_path()),
        ] {
            if !path.is_absolute() {
                bail!("Hive acceptance {name} must be an absolute path");
            }
        }
        validate_disposable_database_path(
            &self.database_path,
            &mitsuro_core::paths::config_dir().join("mitsuro.db"),
        )?;
        reject_workspace_containing_production_state(
            &self.working_dir,
            &mitsuro_core::paths::config_dir().join("mitsuro.db"),
        )?;
        reject_installed_hive_authority(
            &self.hive_socket_path,
            &production_hive_socket_path(),
            "socket",
        )?;
        reject_installed_hive_authority(&self.hive_key_path, &production_hive_key_path(), "key")?;
        if !self.working_dir.is_dir() {
            bail!(
                "Hive acceptance working directory does not exist: {}",
                self.working_dir.display()
            );
        }
        if !self.hive_socket_path.exists() {
            bail!(
                "Hive acceptance socket does not exist: {}",
                self.hive_socket_path.display()
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileTypeExt;

            if !std::fs::metadata(&self.hive_socket_path)
                .with_context(|| {
                    format!(
                        "reading Hive acceptance socket metadata at {}",
                        self.hive_socket_path.display()
                    )
                })?
                .file_type()
                .is_socket()
            {
                bail!(
                    "Hive acceptance socket path is not a Unix socket: {}",
                    self.hive_socket_path.display()
                );
            }
        }
        if !self.hive_key_path.is_file() {
            bail!(
                "Hive acceptance key does not exist: {}",
                self.hive_key_path.display()
            );
        }
        Ok(())
    }
}

/// Start the dedicated loopback-only acceptance server.
///
/// The explicit Hive key is loaded without creation. The application state is
/// built through `IsolatedEvaluation`, which suppresses global plugin, MCP,
/// push/APNs, remote-access, and daemon-discovery side effects; its embedded
/// compatibility runtime is replaced before the router is exposed.
pub async fn start_hive_acceptance_server(config: HiveAcceptanceServerConfig) -> Result<()> {
    config.validate()?;
    let listener = StdTcpListener::bind(config.bind_address)
        .with_context(|| format!("binding Hive acceptance server to {}", config.bind_address))?;
    listener
        .set_nonblocking(true)
        .context("configuring Hive acceptance listener")?;
    let listener =
        tokio::net::TcpListener::from_std(listener).context("adopting Hive acceptance listener")?;
    let local_addr = listener.local_addr()?;
    if !local_addr.ip().is_loopback() {
        bail!("Hive acceptance listener escaped the loopback boundary");
    }

    let hive_runtime = crate::hive_runtime::HiveRuntimeManager::daemon_from_explicit(
        config.hive_socket_path.clone(),
        config.hive_key_path.clone(),
    )
    .await
    .context("connecting to the explicit Hive acceptance daemon")?;

    let server_config = ServerConfig {
        port: local_addr.port(),
        working_dir: config.working_dir,
        database_path: Some(config.database_path.clone()),
    };
    let state = build_app_state_with_hive_runtime(
        &server_config,
        HiveRuntimeMode::IsolatedEvaluation,
        Some(config.database_path),
        Some(hive_runtime),
    )
    .await
    .context("building isolated Hive acceptance state")?;
    let app = build_hive_acceptance_router(state);

    tracing::info!(
        bind_address = %local_addr,
        database_path = %server_config.database_path.as_ref().expect("explicit path").display(),
        hive_socket_path = %config.hive_socket_path.display(),
        "Hive acceptance server listening"
    );
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

fn build_hive_acceptance_router(state: AppState) -> Router {
    let http_policy = ServerHttpPolicy::default();
    let protected_routes = Router::new()
        .nest("/api", routes::hive_acceptance_api_router())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::auth_middleware,
        ));

    Router::new()
        .route("/health", get(health))
        .merge(protected_routes)
        .fallback(serve_web_app)
        .layer(http_policy.cors_layer())
        .layer(TraceLayer::new_for_http())
        .layer(DefaultBodyLimit::max(http_policy.max_request_body_bytes))
        .with_state(state)
}

fn validate_disposable_database_path(candidate: &Path, production: &Path) -> Result<()> {
    if !candidate.is_absolute() {
        bail!("Hive acceptance database_path must be an absolute path");
    }
    let same_file = same_existing_file(candidate, production)?;
    let candidate = path_identity(candidate).context("resolving acceptance database identity")?;
    let production = path_identity(production).context("resolving production database identity")?;
    if candidate == production || same_file {
        bail!(
            "Hive acceptance database must not be the default production database at {}",
            production.display()
        );
    }
    Ok(())
}

fn reject_installed_hive_authority(
    candidate: &Path,
    production: &Path,
    authority: &str,
) -> Result<()> {
    if !production.exists() {
        return Ok(());
    }
    let same_file = same_existing_file(candidate, production)?;
    let candidate = path_identity(candidate)
        .with_context(|| format!("resolving explicit Hive {authority} identity"))?;
    let production = path_identity(production)
        .with_context(|| format!("resolving installed Hive {authority} identity"))?;
    if candidate == production || same_file {
        bail!(
            "Hive acceptance {authority} must not use installed Hive authority at {}",
            production.display()
        );
    }
    Ok(())
}

fn reject_workspace_containing_production_state(workspace: &Path, production: &Path) -> Result<()> {
    if !workspace.is_dir() {
        return Ok(());
    }
    let workspace = std::fs::canonicalize(workspace)
        .with_context(|| format!("canonicalizing {}", workspace.display()))?;
    let production = path_identity(production).context("resolving production database identity")?;
    if production.starts_with(&workspace) {
        bail!(
            "Hive acceptance working_dir must not contain the production database at {}",
            production.display()
        );
    }
    Ok(())
}

fn same_existing_file(left: &Path, right: &Path) -> Result<bool> {
    if !left.exists() || !right.exists() {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        let left = std::fs::metadata(left)
            .with_context(|| format!("reading metadata for {}", left.display()))?;
        let right = std::fs::metadata(right)
            .with_context(|| format!("reading metadata for {}", right.display()))?;
        Ok(left.dev() == right.dev() && left.ino() == right.ino())
    }
    #[cfg(not(unix))]
    Ok(false)
}

fn production_hive_key_path() -> PathBuf {
    std::env::var_os("MITSURO_HIVE_KEY")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            mitsuro_core::paths::config_dir()
                .join("run")
                .join("hive-ipc.key")
        })
}

fn production_hive_socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os("MITSURO_HIVE_SOCKET")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return path;
    }
    if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return runtime_dir.join("mitsuro").join("hive.sock");
    }
    #[cfg(target_os = "macos")]
    if let Some(cache_dir) = dirs::cache_dir() {
        return cache_dir.join("mitsuro").join("run").join("hive.sock");
    }
    std::env::temp_dir()
        .join(format!(
            "mitsuro-{}",
            mitsuro_hive_protocol::current_effective_uid()
        ))
        .join("hive.sock")
}

fn path_identity(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return std::fs::canonicalize(path)
            .with_context(|| format!("canonicalizing {}", path.display()));
    }
    let parent = path
        .parent()
        .context("path has no parent")?
        .canonicalize()
        .with_context(|| format!("canonicalizing parent of {}", path.display()))?;
    let name = path.file_name().context("path has no file name")?;
    Ok(parent.join(name))
}

#[cfg(test)]
mod tests {
    use super::validate_disposable_database_path;

    #[test]
    fn production_database_is_refused_even_through_a_symlink() {
        let temp = tempfile::tempdir().expect("temporary root");
        let production_dir = temp.path().join("production");
        std::fs::create_dir(&production_dir).expect("production directory");
        let production = production_dir.join("mitsuro.db");
        std::fs::write(&production, b"production").expect("production marker");

        #[cfg(unix)]
        let candidate = {
            let alias = temp.path().join("database-alias");
            std::os::unix::fs::symlink(&production, &alias).expect("database symlink");
            alias
        };
        #[cfg(not(unix))]
        let candidate = production.clone();

        let error = validate_disposable_database_path(&candidate, &production)
            .expect_err("production database must fail closed");
        assert!(error.to_string().contains("default production database"));
    }
}
