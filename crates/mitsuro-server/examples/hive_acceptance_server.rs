//! Loopback-only server for live Hive candidate acceptance.
//!
//! This entrypoint requires every authority path explicitly. It never discovers
//! the installed server or Hive daemon and refuses Mitsuro's production DB.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use mitsuro_server::HiveAcceptanceServerConfig;

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
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("mitsuro_server=info,mitsuro_core=info")
            }),
        )
        .with_target(false)
        .with_ansi(false)
        .init();

    let bind_address = std::env::var("MITSURO_HIVE_ACCEPTANCE_BIND")
        .context("MITSURO_HIVE_ACCEPTANCE_BIND must be set")?
        .parse::<SocketAddr>()
        .context("MITSURO_HIVE_ACCEPTANCE_BIND must be an IP socket address")?;
    let config = HiveAcceptanceServerConfig {
        bind_address,
        working_dir: required_path("MITSURO_HIVE_ACCEPTANCE_WORKING_DIR")?,
        database_path: required_path("MITSURO_HIVE_ACCEPTANCE_DATABASE_PATH")?,
        hive_socket_path: required_path("MITSURO_HIVE_ACCEPTANCE_SOCKET_PATH")?,
        hive_key_path: required_path("MITSURO_HIVE_ACCEPTANCE_KEY_PATH")?,
    };
    mitsuro_server::start_hive_acceptance_server(config).await
}
