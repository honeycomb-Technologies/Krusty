//! Server instance detection and management
//!
//! Tracks running Mitsuro server instances via a PID file at ~/.mitsuro/server.pid.
//! Enables the desktop app and CLI to detect and reuse an existing server.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

use crate::paths;

pub const HEALTH_IDENTITY: &str = "mitsuro-server";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInstance {
    pub pid: u32,
    pub port: u16,
}

#[derive(Debug, Deserialize)]
struct HealthIdentity {
    identity: String,
    pid: u32,
}

fn pid_file_path() -> PathBuf {
    paths::config_dir().join("server.pid")
}

/// Write PID file when server starts.
pub fn write_pid_file(port: u16) -> Result<()> {
    let path = pid_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = format!("{}:{}", std::process::id(), port);
    std::fs::write(&path, content).context("Failed to write server PID file")?;
    Ok(())
}

/// Remove a PID file on shutdown only when it still belongs to this process.
/// New code that knows the port should prefer [`remove_pid_file_if_matches`].
pub fn remove_pid_file() {
    let path = pid_file_path();
    let Some(instance) = read_pid_file_from(&path, false) else {
        return;
    };
    if instance.pid == std::process::id() {
        let _ = remove_pid_file_if_matches_at(&path, &instance);
    }
}

/// Remove the PID file only if it still contains the exact expected PID and
/// port. This prevents a stale health probe from unlinking a replacement
/// server's authority record.
pub fn remove_pid_file_if_matches(expected_pid: u32, expected_port: u16) -> bool {
    remove_pid_file_if_matches_at(
        &pid_file_path(),
        &ServerInstance {
            pid: expected_pid,
            port: expected_port,
        },
    )
}

/// Read PID file and check if the process is still alive.
pub fn read_pid_file() -> Option<ServerInstance> {
    let path = pid_file_path();
    read_pid_file_from(&path, true)
}

fn read_pid_file_from(path: &std::path::Path, clean_stale: bool) -> Option<ServerInstance> {
    let content = std::fs::read_to_string(path).ok()?;
    let instance = parse_pid_file(&content)?;

    // Check if process is alive (Unix: kill -0)
    if !is_process_alive(instance.pid) {
        if clean_stale {
            let _ = remove_pid_file_if_matches_at(path, &instance);
        }
        return None;
    }

    Some(instance)
}

fn parse_pid_file(content: &str) -> Option<ServerInstance> {
    let (pid, port) = content.trim().split_once(':')?;
    if port.contains(':') {
        return None;
    }
    Some(ServerInstance {
        pid: pid.parse().ok()?,
        port: port.parse().ok()?,
    })
}

fn remove_pid_file_if_matches_at(path: &std::path::Path, expected: &ServerInstance) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    if parse_pid_file(&content).as_ref() != Some(expected) {
        return false;
    }
    std::fs::remove_file(path).is_ok()
}

/// Check whether this process hosts the canonical Mitsuro server on `port`.
/// This is used while an embedded server is starting.
pub async fn probe_health(port: u16) -> bool {
    probe_health_for_instance(&ServerInstance {
        pid: std::process::id(),
        port,
    })
    .await
}

async fn probe_health_for_instance(instance: &ServerInstance) -> bool {
    let url = format!("http://127.0.0.1:{}/health", instance.port);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build();

    let client = match client {
        Ok(c) => c,
        Err(_) => return false,
    };

    let Ok(response) = client.get(&url).send().await else {
        return false;
    };
    if !response.status().is_success() {
        return false;
    }
    response
        .json::<HealthIdentity>()
        .await
        .is_ok_and(|health| health_identity_matches(instance, &health))
}

fn health_identity_matches(instance: &ServerInstance, health: &HealthIdentity) -> bool {
    health.identity == HEALTH_IDENTITY && health.pid == instance.pid
}

/// Detect a running Mitsuro server instance.
/// Returns the instance info if a healthy server is found.
pub async fn detect_running_server() -> Option<ServerInstance> {
    let instance = read_pid_file()?;

    if !process_can_host_canonical_server(instance.pid) {
        let _ = remove_pid_file_if_matches(instance.pid, instance.port);
        return None;
    }

    if probe_health_for_instance(&instance).await {
        Some(instance)
    } else {
        // Process alive but not the exact canonical server recorded by this
        // PID file. Remove only the authority record we actually inspected.
        let _ = remove_pid_file_if_matches(instance.pid, instance.port);
        None
    }
}

#[cfg(target_os = "linux")]
fn process_can_host_canonical_server(pid: u32) -> bool {
    let path = PathBuf::from(format!("/proc/{pid}/exe"));
    std::fs::read_link(path)
        .map(strip_deleted_suffix)
        .is_ok_and(|path| is_canonical_server_host_executable(&path))
}

#[cfg(not(target_os = "linux"))]
fn process_can_host_canonical_server(_pid: u32) -> bool {
    // The health response still has to prove canonical identity and the exact
    // PID. Linux additionally provides a trustworthy executable path via
    // procfs; other platforms do not expose an equivalent through std.
    true
}

#[cfg(target_os = "linux")]
fn strip_deleted_suffix(path: PathBuf) -> PathBuf {
    path.to_string_lossy()
        .strip_suffix(" (deleted)")
        .map_or(path.clone(), PathBuf::from)
}

fn is_canonical_server_host_executable(path: &std::path::Path) -> bool {
    if path
        .components()
        .any(|component| component.as_os_str() == std::ffi::OsStr::new(".mitsuro-releases"))
    {
        return true;
    }
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(
            "mitsuro"
                | "mitsuro.exe"
                | "mitsuro-server"
                | "mitsuro-server.exe"
                | "mitsuro-desktop"
                | "mitsuro-desktop.exe"
                | "Mitsuro"
                | "Mitsuro.exe"
        )
    )
}

#[cfg(unix)]
fn is_process_alive(pid: u32) -> bool {
    if pid > i32::MAX as u32 {
        return false;
    }
    // SAFETY: kill(pid, 0) with signal 0 only checks process existence
    // without sending a signal. The pid is guarded to fit in i32.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(not(unix))]
fn is_process_alive(_pid: u32) -> bool {
    // On non-Unix, assume alive and let health check determine
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_file_parser_requires_exact_pid_and_port_shape() {
        assert_eq!(
            parse_pid_file("123:4567"),
            Some(ServerInstance {
                pid: 123,
                port: 4567
            })
        );
        for invalid in ["", "123", "123:4567:extra", "pid:4567", "123:70000"] {
            assert_eq!(parse_pid_file(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn conditional_cleanup_never_removes_a_replacement_pid_file() {
        let temp = tempfile::tempdir().expect("temp dir");
        let path = temp.path().join("server.pid");
        std::fs::write(&path, "200:4000").expect("replacement PID file");
        assert!(!remove_pid_file_if_matches_at(
            &path,
            &ServerInstance {
                pid: 100,
                port: 3000,
            }
        ));
        assert_eq!(
            std::fs::read_to_string(&path).expect("replacement remains"),
            "200:4000"
        );
        assert!(remove_pid_file_if_matches_at(
            &path,
            &ServerInstance {
                pid: 200,
                port: 4000,
            }
        ));
        assert!(!path.exists());
    }

    #[test]
    fn health_identity_requires_canonical_role_and_exact_pid() {
        let expected = ServerInstance {
            pid: 123,
            port: 3000,
        };
        assert!(health_identity_matches(
            &expected,
            &HealthIdentity {
                identity: HEALTH_IDENTITY.to_string(),
                pid: 123,
            }
        ));
        assert!(!health_identity_matches(
            &expected,
            &HealthIdentity {
                identity: "krusty-server".to_string(),
                pid: 123,
            }
        ));
        assert!(!health_identity_matches(
            &expected,
            &HealthIdentity {
                identity: HEALTH_IDENTITY.to_string(),
                pid: 999,
            }
        ));
    }

    #[test]
    fn executable_check_accepts_only_canonical_server_hosts() {
        for accepted in [
            "/opt/mitsuro",
            "/opt/mitsuro-server",
            "/opt/mitsuro-desktop",
            "/srv/.mitsuro-releases/0.9.20/arbitrary-name",
        ] {
            assert!(is_canonical_server_host_executable(std::path::Path::new(
                accepted
            )));
        }
        for rejected in [
            "/opt/krusty",
            "/opt/krusty-mako",
            "/opt/mitsuro-helper",
            "/srv/not-.mitsuro-releases/arbitrary-name",
        ] {
            assert!(!is_canonical_server_host_executable(std::path::Path::new(
                rejected
            )));
        }
    }
}
