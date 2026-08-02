//! Centralized path utilities
//!
//! All application paths in one place for consistency

use std::path::{Path, PathBuf};

use crate::{constants::ui, identity::CONFIG_DIR_NAME};

pub const HIVE_SOUL_FILE: &str = "HIVE_SOUL.md";
pub const HIVE_IDENTITY_FILE: &str = "HIVE_IDENTITY.md";
pub const HIVE_USER_FILE: &str = "HIVE_USER.md";
pub const HIVE_HEARTBEAT_FILE: &str = "HIVE_HEARTBEAT.md";
pub const HIVE_MEMORY_FILE: &str = "HIVE_MEMORY.md";
pub const HIVE_CHANNELS_FILE: &str = "HIVE_CHANNELS.md";

/// Get the mitsuro config directory (~/.mitsuro)
pub fn config_dir() -> PathBuf {
    if running_under_cargo_test() {
        return isolated_test_config_dir();
    }

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(CONFIG_DIR_NAME)
}

/// Cargo builds integration-test dependencies without `cfg(test)`, so the
/// compile-time flag alone cannot protect the real user config directory. Test
/// harness executables have a stable `target/<profile>/deps/<name>-<hash>`
/// shape; rustdoc uses a `rustdoctest*` temporary directory.
fn running_under_cargo_test() -> bool {
    if cfg!(test) {
        return true;
    }

    std::env::current_exe()
        .map(|path| executable_looks_like_test_harness(&path))
        .unwrap_or(false)
}

fn executable_looks_like_test_harness(executable: &Path) -> bool {
    let is_rustdoc_test = executable.components().any(|component| {
        component
            .as_os_str()
            .to_string_lossy()
            .starts_with("rustdoctest")
    });
    if is_rustdoc_test {
        return true;
    }

    let Some(parent_name) = executable
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
    else {
        return false;
    };
    let Some(stem) = executable.file_stem().and_then(|stem| stem.to_str()) else {
        return false;
    };
    let Some((_, hash)) = stem.rsplit_once('-') else {
        return false;
    };

    parent_name == "deps" && hash.len() >= 16 && hash.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn isolated_test_config_dir() -> PathBuf {
    static TEST_CONFIG_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

    TEST_CONFIG_DIR
        .get_or_init(|| {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            std::env::temp_dir()
                .join("mitsuro-cargo-tests")
                .join(format!("{}-{nonce}", std::process::id()))
                .join(CONFIG_DIR_NAME)
        })
        .clone()
}

/// Get the mitsuro config directory for a specific home/root path (<home>/.mitsuro)
pub fn config_dir_for_home(home_dir: &Path) -> PathBuf {
    home_dir.join(CONFIG_DIR_NAME)
}

/// Get the per-project Mitsuro state directory (<project>/.mitsuro)
pub fn project_state_dir(project_root: &Path) -> PathBuf {
    project_root.join(CONFIG_DIR_NAME)
}

/// Get the per-project reports directory (<project>/.mitsuro/reports)
pub fn project_reports_dir(project_root: &Path) -> PathBuf {
    project_state_dir(project_root).join("reports")
}

/// Get the global Hive home directory (~/.mitsuro/hive)
pub fn hive_dir() -> PathBuf {
    config_dir().join("hive")
}

/// Get the Hive home directory for a specific home/root path (<home>/.mitsuro/hive)
pub fn hive_dir_for_home(home_dir: &Path) -> PathBuf {
    config_dir_for_home(home_dir).join("hive")
}

/// Get the global Hive crew directory (~/.mitsuro/hive/crew)
pub fn hive_crew_dir() -> PathBuf {
    hive_dir().join("crew")
}

/// Get the Hive crew directory for a specific home/root path (<home>/.mitsuro/hive/crew)
pub fn hive_crew_dir_for_home(home_dir: &Path) -> PathBuf {
    hive_dir_for_home(home_dir).join("crew")
}

/// Get a file path inside the global Hive home (~/.mitsuro/hive/<name>)
pub fn hive_file_path(name: &str) -> PathBuf {
    hive_dir().join(name)
}

/// Get a file path inside a specific Hive home (<home>/.mitsuro/hive/<name>)
pub fn hive_file_path_for_home(home_dir: &Path, name: &str) -> PathBuf {
    hive_dir_for_home(home_dir).join(name)
}

pub fn hive_soul_path() -> PathBuf {
    hive_file_path(HIVE_SOUL_FILE)
}

pub fn hive_identity_path() -> PathBuf {
    hive_file_path(HIVE_IDENTITY_FILE)
}

pub fn hive_user_path() -> PathBuf {
    hive_file_path(HIVE_USER_FILE)
}

pub fn hive_heartbeat_path() -> PathBuf {
    hive_file_path(HIVE_HEARTBEAT_FILE)
}

pub fn hive_memory_path() -> PathBuf {
    hive_file_path(HIVE_MEMORY_FILE)
}

pub fn hive_channels_path() -> PathBuf {
    hive_file_path(HIVE_CHANNELS_FILE)
}

/// Ensure the global Hive home exists, creating it if necessary.
pub fn ensure_hive_dir() -> std::io::Result<PathBuf> {
    let dir = hive_dir();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn hive_crew_member_dir(member_slug: &str) -> PathBuf {
    hive_crew_dir().join(member_slug)
}

pub fn hive_crew_member_dir_for_home(home_dir: &Path, member_slug: &str) -> PathBuf {
    hive_crew_dir_for_home(home_dir).join(member_slug)
}

pub fn hive_crew_member_file_path(member_slug: &str, file_name: &str) -> PathBuf {
    hive_crew_member_dir(member_slug).join(file_name)
}

pub fn hive_crew_member_file_path_for_home(
    home_dir: &Path,
    member_slug: &str,
    file_name: &str,
) -> PathBuf {
    hive_crew_member_dir_for_home(home_dir, member_slug).join(file_name)
}

/// Get the extensions directory (~/.mitsuro/extensions)
pub fn extensions_dir() -> PathBuf {
    config_dir().join(ui::EXTENSIONS_DIR_NAME)
}

/// Get the installable plugins directory (~/.mitsuro/plugins)
pub fn plugins_dir() -> PathBuf {
    config_dir().join(ui::PLUGINS_DIR_NAME)
}

/// Get the logs directory (~/.mitsuro/logs)
pub fn logs_dir() -> PathBuf {
    config_dir().join("logs")
}

/// Get the tokens directory (~/.mitsuro/tokens)
pub fn tokens_dir() -> PathBuf {
    config_dir().join("tokens")
}

/// Get the plans directory (~/.mitsuro/plans)
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

/// Get the MCP keys file (~/.mitsuro/tokens/mcp_keys.json)
/// Used for storing API keys for MCP servers
pub fn mcp_keys_path() -> PathBuf {
    tokens_dir().join("mcp_keys.json")
}

/// Get the VAPID key file (~/.mitsuro/tokens/vapid_key.pem)
/// Used for Web Push notification authentication.
/// Auto-generated on first server startup if absent.
pub fn vapid_key_path() -> PathBuf {
    tokens_dir().join("vapid_key.pem")
}

#[cfg(test)]
mod tests {
    use super::{
        config_dir, config_dir_for_home, executable_looks_like_test_harness, hive_channels_path,
        hive_crew_dir, hive_crew_dir_for_home, hive_crew_member_dir, hive_crew_member_dir_for_home,
        hive_crew_member_file_path, hive_crew_member_file_path_for_home, hive_dir,
        hive_dir_for_home, hive_file_path, hive_file_path_for_home, hive_heartbeat_path,
        hive_identity_path, hive_memory_path, hive_soul_path, hive_user_path, CONFIG_DIR_NAME,
        HIVE_CHANNELS_FILE, HIVE_HEARTBEAT_FILE, HIVE_IDENTITY_FILE, HIVE_MEMORY_FILE,
        HIVE_SOUL_FILE, HIVE_USER_FILE,
    };
    use std::path::{Path, PathBuf};

    #[test]
    fn global_config_dir_is_isolated_during_unit_tests() {
        let config = config_dir();
        assert!(config.starts_with(std::env::temp_dir().join("mitsuro-cargo-tests")));
        assert_eq!(
            config.file_name(),
            Some(std::ffi::OsStr::new(CONFIG_DIR_NAME))
        );
        if let Some(home) = dirs::home_dir() {
            assert_ne!(config, home.join(CONFIG_DIR_NAME));
        }
    }

    #[test]
    fn recognizes_cargo_test_harness_executables_only() {
        assert!(executable_looks_like_test_harness(Path::new(
            "/workspace/target/debug/deps/path_isolation-0123456789abcdef"
        )));
        assert!(executable_looks_like_test_harness(Path::new(
            "/tmp/rustdoctestABCD/rust_out"
        )));
        assert!(!executable_looks_like_test_harness(Path::new(
            "/workspace/target/debug/mitsuro"
        )));
        assert!(!executable_looks_like_test_harness(&PathBuf::from(
            "/workspace/target/debug/deps/mitsuro"
        )));
    }

    #[test]
    fn hive_paths_live_under_global_config_dir() {
        let config = config_dir();
        assert_eq!(hive_dir(), config.join("hive"));
        assert_eq!(hive_crew_dir(), config.join("hive").join("crew"));
        assert_eq!(hive_soul_path(), config.join("hive").join(HIVE_SOUL_FILE));
        assert_eq!(
            hive_identity_path(),
            config.join("hive").join(HIVE_IDENTITY_FILE)
        );
        assert_eq!(hive_user_path(), config.join("hive").join(HIVE_USER_FILE));
        assert_eq!(
            hive_heartbeat_path(),
            config.join("hive").join(HIVE_HEARTBEAT_FILE)
        );
        assert_eq!(
            hive_memory_path(),
            config.join("hive").join(HIVE_MEMORY_FILE)
        );
        assert_eq!(
            hive_channels_path(),
            config.join("hive").join(HIVE_CHANNELS_FILE)
        );
        assert_eq!(
            hive_file_path(HIVE_SOUL_FILE),
            config.join("hive").join(HIVE_SOUL_FILE)
        );
    }

    #[test]
    fn crew_member_paths_live_under_global_hive_crew_dir() {
        let config = config_dir();
        assert_eq!(
            hive_crew_member_dir("researcher"),
            config.join("hive").join("crew").join("researcher")
        );
        assert_eq!(
            hive_crew_member_file_path("researcher", "SOUL.md"),
            config
                .join("hive")
                .join("crew")
                .join("researcher")
                .join("SOUL.md")
        );
    }

    #[test]
    fn scoped_hive_paths_live_under_supplied_home_dir() {
        let home = Path::new("/tmp/hive-user");
        assert_eq!(config_dir_for_home(home), home.join(".mitsuro"));
        assert_eq!(hive_dir_for_home(home), home.join(".mitsuro").join("hive"));
        assert_eq!(
            hive_crew_dir_for_home(home),
            home.join(".mitsuro").join("hive").join("crew")
        );
        assert_eq!(
            hive_file_path_for_home(home, HIVE_SOUL_FILE),
            home.join(".mitsuro").join("hive").join(HIVE_SOUL_FILE)
        );
        assert_eq!(
            hive_crew_member_dir_for_home(home, "reviewer"),
            home.join(".mitsuro")
                .join("hive")
                .join("crew")
                .join("reviewer")
        );
        assert_eq!(
            hive_crew_member_file_path_for_home(home, "reviewer", "SOUL.md"),
            home.join(".mitsuro")
                .join("hive")
                .join("crew")
                .join("reviewer")
                .join("SOUL.md")
        );
    }
}
