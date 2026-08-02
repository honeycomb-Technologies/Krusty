use std::path::PathBuf;
use std::time::Duration;

use mitsuro_hive_protocol::{current_effective_uid, AuthPolicy};

#[derive(Debug, Clone)]
pub struct HivePaths {
    pub socket_path: PathBuf,
    pub key_path: PathBuf,
}

impl HivePaths {
    pub fn discover() -> anyhow::Result<Self> {
        let socket_path = env_path("MITSURO_HIVE_SOCKET").unwrap_or_else(default_socket_path);
        let key_path = env_path("MITSURO_HIVE_KEY").unwrap_or_else(default_key_path);
        Ok(Self {
            socket_path,
            key_path,
        })
    }
}

#[derive(Debug, Clone)]
pub struct HiveDaemonConfig {
    pub paths: HivePaths,
    pub instance_id: String,
    pub auth_policy: AuthPolicy,
    /// Bounds unauthenticated sockets and stalled response writers.
    pub control_io_timeout: Duration,
    pub connection_grace_period: Duration,
    pub max_connections: usize,
}

impl HiveDaemonConfig {
    pub fn discover() -> anyhow::Result<Self> {
        Ok(Self {
            paths: HivePaths::discover()?,
            instance_id: uuid::Uuid::new_v4().to_string(),
            auth_policy: AuthPolicy::default(),
            control_io_timeout: Duration::from_secs(5),
            connection_grace_period: Duration::from_secs(10),
            max_connections: 128,
        })
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    mitsuro_core::identity::env_var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn default_socket_path() -> PathBuf {
    if let Some(runtime_dir) = env_path("XDG_RUNTIME_DIR") {
        return runtime_dir.join("mitsuro").join("hive.sock");
    }

    #[cfg(target_os = "macos")]
    if let Some(cache_dir) = dirs::cache_dir() {
        return cache_dir.join("mitsuro").join("run").join("hive.sock");
    }

    std::env::temp_dir()
        .join(format!("mitsuro-{}", current_effective_uid()))
        .join("hive.sock")
}

fn default_key_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".mitsuro")
        .join("run")
        .join("hive-ipc.key")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_paths_are_preserved() {
        let paths = HivePaths {
            socket_path: PathBuf::from("/tmp/example.sock"),
            key_path: PathBuf::from("/tmp/example.key"),
        };
        assert_eq!(paths.socket_path, PathBuf::from("/tmp/example.sock"));
        assert_eq!(paths.key_path, PathBuf::from("/tmp/example.key"));
    }
}
