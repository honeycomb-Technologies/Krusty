//! Centralized path utilities
//!
//! All application paths in one place for consistency

use std::path::{Path, PathBuf};

use crate::constants::ui;

/// Get the krusty config directory (~/.krusty)
pub fn config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(ui::CONFIG_DIR_NAME)
}

/// Get the per-project Krusty state directory (<project>/.krusty)
pub fn project_state_dir(project_root: &Path) -> PathBuf {
    project_root.join(ui::CONFIG_DIR_NAME)
}

/// Get the per-project reports directory (<project>/.krusty/reports)
pub fn project_reports_dir(project_root: &Path) -> PathBuf {
    project_state_dir(project_root).join("reports")
}

/// Get the extensions directory (~/.krusty/extensions)
pub fn extensions_dir() -> PathBuf {
    config_dir().join(ui::EXTENSIONS_DIR_NAME)
}

/// Get the installable plugins directory (~/.krusty/plugins)
pub fn plugins_dir() -> PathBuf {
    config_dir().join(ui::PLUGINS_DIR_NAME)
}

/// Get the logs directory (~/.krusty/logs)
pub fn logs_dir() -> PathBuf {
    config_dir().join("logs")
}

/// Get the tokens directory (~/.krusty/tokens)
pub fn tokens_dir() -> PathBuf {
    config_dir().join("tokens")
}

/// Get the plans directory (~/.krusty/plans)
/// Used for storing plan files in plan mode
pub fn plans_dir() -> PathBuf {
    config_dir().join("plans")
}

/// Ensure the plans directory exists, creating it if necessary
pub fn ensure_plans_dir() -> std::io::Result<PathBuf> {
    let dir = plans_dir();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Get the MCP keys file (~/.krusty/tokens/mcp_keys.json)
/// Used for storing API keys for MCP servers
pub fn mcp_keys_path() -> PathBuf {
    tokens_dir().join("mcp_keys.json")
}

/// Get the VAPID key file (~/.krusty/tokens/vapid_key.pem)
/// Used for Web Push notification authentication.
/// Auto-generated on first server startup if absent.
pub fn vapid_key_path() -> PathBuf {
    tokens_dir().join("vapid_key.pem")
}
