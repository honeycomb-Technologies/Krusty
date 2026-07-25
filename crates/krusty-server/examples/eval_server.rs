//! Isolated server entrypoint for live candidate evaluation.
//!
//! This intentionally bypasses the installed CLI's shared PID-file discovery
//! while retaining the normal server router, credentials, model catalog, and
//! agent runtime. Its database and workspace must be explicit disposable paths.

use std::net::Ipv4Addr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use krusty_server::ServerConfig;

fn required_path(name: &str) -> Result<PathBuf> {
    let value = std::env::var(name).with_context(|| format!("{name} must be set"))?;
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        anyhow::bail!("{name} must be an absolute path");
    }
    Ok(path)
}

#[tokio::main]
async fn main() -> Result<()> {
    let port = std::env::var("KRUSTY_EVAL_PORT")
        .unwrap_or_else(|_| "3100".to_string())
        .parse::<u16>()
        .context("KRUSTY_EVAL_PORT must be a valid TCP port")?;
    let working_dir = required_path("KRUSTY_EVAL_WORKING_DIR")?;
    let database_path = required_path("KRUSTY_EVAL_DATABASE_PATH")?;

    std::fs::create_dir_all(&working_dir)
        .with_context(|| format!("creating {}", working_dir.display()))?;
    if let Some(parent) = database_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }

    let config = ServerConfig {
        port,
        working_dir,
        database_path: Some(database_path),
    };
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port))
        .await
        .with_context(|| format!("binding isolated evaluation server to 127.0.0.1:{port}"))?;

    krusty_server::start_isolated_server_with_listener(config, listener).await
}
