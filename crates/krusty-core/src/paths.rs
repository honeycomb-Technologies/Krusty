//! Centralized path utilities
//!
//! All application paths in one place for consistency

use std::path::{Path, PathBuf};

use crate::constants::ui;

pub const MAKO_SOUL_FILE: &str = "MAKO_SOUL.md";
pub const MAKO_IDENTITY_FILE: &str = "MAKO_IDENTITY.md";
pub const MAKO_USER_FILE: &str = "MAKO_USER.md";
pub const MAKO_HEARTBEAT_FILE: &str = "MAKO_HEARTBEAT.md";
pub const MAKO_MEMORY_FILE: &str = "MAKO_MEMORY.md";
pub const MAKO_CHANNELS_FILE: &str = "MAKO_CHANNELS.md";

/// Get the krusty config directory (~/.krusty)
pub fn config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(ui::CONFIG_DIR_NAME)
}

/// Get the krusty config directory for a specific home/root path (<home>/.krusty)
pub fn config_dir_for_home(home_dir: &Path) -> PathBuf {
    home_dir.join(ui::CONFIG_DIR_NAME)
}

/// Get the per-project Krusty state directory (<project>/.krusty)
pub fn project_state_dir(project_root: &Path) -> PathBuf {
    project_root.join(ui::CONFIG_DIR_NAME)
}

/// Get the per-project reports directory (<project>/.krusty/reports)
pub fn project_reports_dir(project_root: &Path) -> PathBuf {
    project_state_dir(project_root).join("reports")
}

/// Get the global Mako home directory (~/.krusty/mako)
pub fn mako_dir() -> PathBuf {
    config_dir().join("mako")
}

/// Get the Mako home directory for a specific home/root path (<home>/.krusty/mako)
pub fn mako_dir_for_home(home_dir: &Path) -> PathBuf {
    config_dir_for_home(home_dir).join("mako")
}

/// Get the global Mako crew directory (~/.krusty/mako/crew)
pub fn mako_crew_dir() -> PathBuf {
    mako_dir().join("crew")
}

/// Get the Mako crew directory for a specific home/root path (<home>/.krusty/mako/crew)
pub fn mako_crew_dir_for_home(home_dir: &Path) -> PathBuf {
    mako_dir_for_home(home_dir).join("crew")
}

/// Get a file path inside the global Mako home (~/.krusty/mako/<name>)
pub fn mako_file_path(name: &str) -> PathBuf {
    mako_dir().join(name)
}

/// Get a file path inside a specific Mako home (<home>/.krusty/mako/<name>)
pub fn mako_file_path_for_home(home_dir: &Path, name: &str) -> PathBuf {
    mako_dir_for_home(home_dir).join(name)
}

pub fn mako_soul_path() -> PathBuf {
    mako_file_path(MAKO_SOUL_FILE)
}

pub fn mako_identity_path() -> PathBuf {
    mako_file_path(MAKO_IDENTITY_FILE)
}

pub fn mako_user_path() -> PathBuf {
    mako_file_path(MAKO_USER_FILE)
}

pub fn mako_heartbeat_path() -> PathBuf {
    mako_file_path(MAKO_HEARTBEAT_FILE)
}

pub fn mako_memory_path() -> PathBuf {
    mako_file_path(MAKO_MEMORY_FILE)
}

pub fn mako_channels_path() -> PathBuf {
    mako_file_path(MAKO_CHANNELS_FILE)
}

/// Ensure the global Mako home exists, creating it if necessary.
pub fn ensure_mako_dir() -> std::io::Result<PathBuf> {
    let dir = mako_dir();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn mako_crew_member_dir(member_slug: &str) -> PathBuf {
    mako_crew_dir().join(member_slug)
}

pub fn mako_crew_member_dir_for_home(home_dir: &Path, member_slug: &str) -> PathBuf {
    mako_crew_dir_for_home(home_dir).join(member_slug)
}

pub fn mako_crew_member_file_path(member_slug: &str, file_name: &str) -> PathBuf {
    mako_crew_member_dir(member_slug).join(file_name)
}

pub fn mako_crew_member_file_path_for_home(
    home_dir: &Path,
    member_slug: &str,
    file_name: &str,
) -> PathBuf {
    mako_crew_member_dir_for_home(home_dir, member_slug).join(file_name)
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

#[cfg(test)]
mod tests {
    use super::{
        config_dir, config_dir_for_home, mako_channels_path, mako_crew_dir, mako_crew_dir_for_home,
        mako_crew_member_dir, mako_crew_member_dir_for_home, mako_crew_member_file_path,
        mako_crew_member_file_path_for_home, mako_dir, mako_dir_for_home, mako_file_path,
        mako_file_path_for_home, mako_heartbeat_path, mako_identity_path, mako_memory_path,
        mako_soul_path, mako_user_path, MAKO_CHANNELS_FILE, MAKO_HEARTBEAT_FILE,
        MAKO_IDENTITY_FILE, MAKO_MEMORY_FILE, MAKO_SOUL_FILE, MAKO_USER_FILE,
    };
    use std::path::Path;

    #[test]
    fn mako_paths_live_under_global_config_dir() {
        let config = config_dir();
        assert_eq!(mako_dir(), config.join("mako"));
        assert_eq!(mako_crew_dir(), config.join("mako").join("crew"));
        assert_eq!(mako_soul_path(), config.join("mako").join(MAKO_SOUL_FILE));
        assert_eq!(
            mako_identity_path(),
            config.join("mako").join(MAKO_IDENTITY_FILE)
        );
        assert_eq!(mako_user_path(), config.join("mako").join(MAKO_USER_FILE));
        assert_eq!(
            mako_heartbeat_path(),
            config.join("mako").join(MAKO_HEARTBEAT_FILE)
        );
        assert_eq!(
            mako_memory_path(),
            config.join("mako").join(MAKO_MEMORY_FILE)
        );
        assert_eq!(
            mako_channels_path(),
            config.join("mako").join(MAKO_CHANNELS_FILE)
        );
        assert_eq!(
            mako_file_path(MAKO_SOUL_FILE),
            config.join("mako").join(MAKO_SOUL_FILE)
        );
    }

    #[test]
    fn crew_member_paths_live_under_global_mako_crew_dir() {
        let config = config_dir();
        assert_eq!(
            mako_crew_member_dir("researcher"),
            config.join("mako").join("crew").join("researcher")
        );
        assert_eq!(
            mako_crew_member_file_path("researcher", "SOUL.md"),
            config
                .join("mako")
                .join("crew")
                .join("researcher")
                .join("SOUL.md")
        );
    }

    #[test]
    fn scoped_mako_paths_live_under_supplied_home_dir() {
        let home = Path::new("/tmp/mako-user");
        assert_eq!(config_dir_for_home(home), home.join(".krusty"));
        assert_eq!(mako_dir_for_home(home), home.join(".krusty").join("mako"));
        assert_eq!(
            mako_crew_dir_for_home(home),
            home.join(".krusty").join("mako").join("crew")
        );
        assert_eq!(
            mako_file_path_for_home(home, MAKO_SOUL_FILE),
            home.join(".krusty").join("mako").join(MAKO_SOUL_FILE)
        );
        assert_eq!(
            mako_crew_member_dir_for_home(home, "reviewer"),
            home.join(".krusty")
                .join("mako")
                .join("crew")
                .join("reviewer")
        );
        assert_eq!(
            mako_crew_member_file_path_for_home(home, "reviewer", "SOUL.md"),
            home.join(".krusty")
                .join("mako")
                .join("crew")
                .join("reviewer")
                .join("SOUL.md")
        );
    }
}
