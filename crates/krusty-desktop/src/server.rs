use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Context as _, Result};

use crate::api::KrustyApiClient;

const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:3000";
const SERVER_START_ATTEMPTS: usize = 50;
const SERVER_START_POLL_INTERVAL: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerEnsureResult {
    pub base_url: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PidFileInstance {
    pid: u32,
    port: u16,
}

pub fn default_server_url() -> &'static str {
    DEFAULT_SERVER_URL
}

pub fn ensure_local_server(preferred_url: String) -> Result<ServerEnsureResult> {
    let preferred = normalized_or_default(preferred_url);
    if health_ok(&preferred) {
        return Ok(ServerEnsureResult {
            base_url: preferred.clone(),
            detail: format!("Connected to configured server at {preferred}."),
        });
    }

    if let Some(instance) = detect_running_server() {
        let base_url = url_for_port(instance.port);
        return Ok(ServerEnsureResult {
            base_url,
            detail: format!(
                "Reused running Krusty server on port {} (pid {}).",
                instance.port, instance.pid
            ),
        });
    }

    let log_path = server_log_path();
    let mut child = spawn_server_process(&log_path).with_context(|| {
        format!(
            "failed to start `krusty serve`; try running it manually. Log path: {}",
            log_path.display()
        )
    })?;

    for _ in 0..SERVER_START_ATTEMPTS {
        thread::sleep(SERVER_START_POLL_INTERVAL);

        if let Some(instance) = detect_running_server() {
            let base_url = url_for_port(instance.port);
            return Ok(ServerEnsureResult {
                base_url,
                detail: format!(
                    "Started Krusty server on port {} (pid {}).",
                    instance.port, instance.pid
                ),
            });
        }

        if health_ok(DEFAULT_SERVER_URL) {
            return Ok(ServerEnsureResult {
                base_url: DEFAULT_SERVER_URL.to_owned(),
                detail: "Started Krusty server on the default port.".to_owned(),
            });
        }

        if let Some(status) = child
            .try_wait()
            .context("failed to poll Krusty server process")?
        {
            return Err(anyhow!(
                "`krusty serve` exited before becoming healthy ({status}). Check {}. If this is first run, run `krusty serve` in a terminal to complete provider setup.",
                log_path.display()
            ));
        }
    }

    Err(anyhow!(
        "Timed out waiting for `krusty serve` to become healthy. Check {} or run `krusty serve` manually.",
        log_path.display()
    ))
}

fn detect_running_server() -> Option<PidFileInstance> {
    let instance = read_pid_file()?;
    if !process_alive(instance.pid) {
        remove_pid_file();
        return None;
    }

    let base_url = url_for_port(instance.port);
    if health_ok(&base_url) {
        Some(instance)
    } else {
        remove_pid_file();
        None
    }
}

fn health_ok(base_url: &str) -> bool {
    KrustyApiClient::new(base_url).health().is_ok()
}

fn read_pid_file() -> Option<PidFileInstance> {
    let raw = fs::read_to_string(pid_file_path()).ok()?;
    let (pid, port) = raw.trim().split_once(':')?;
    Some(PidFileInstance {
        pid: pid.parse().ok()?,
        port: port.parse().ok()?,
    })
}

fn remove_pid_file() {
    let _ = fs::remove_file(pid_file_path());
}

fn pid_file_path() -> PathBuf {
    krusty_config_dir().join("server.pid")
}

fn server_log_path() -> PathBuf {
    krusty_config_dir().join("krusty-desktop-server.log")
}

fn krusty_config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".krusty")
}

fn spawn_server_process(log_path: &PathBuf) -> Result<Child> {
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let first_error = match spawn_command("krusty", &["serve"], None, log_path) {
        Ok(child) => return Ok(child),
        Err(error) => error,
    };

    let workspace = workspace_root();
    spawn_command(
        "cargo",
        &["run", "-p", "krusty", "--", "serve"],
        Some(workspace),
        log_path,
    )
    .with_context(|| format!("`krusty serve` failed to spawn: {first_error}"))
}

fn spawn_command(
    program: &str,
    args: &[&str],
    current_dir: Option<PathBuf>,
    log_path: &PathBuf,
) -> std::io::Result<Child> {
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
    let stderr = stdout.try_clone()?;
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    if let Some(current_dir) = current_dir {
        command.current_dir(current_dir);
    }
    command.spawn()
}

fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|path| path.parent())
        .map(PathBuf::from)
        .unwrap_or(manifest)
}

fn url_for_port(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

fn normalized_or_default(value: String) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        DEFAULT_SERVER_URL.to_owned()
    } else {
        trimmed.trim_end_matches('/').to_owned()
    }
}

#[cfg(target_os = "linux")]
fn process_alive(pid: u32) -> bool {
    PathBuf::from(format!("/proc/{pid}")).exists()
}

#[cfg(all(unix, not(target_os = "linux")))]
fn process_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn process_alive(_pid: u32) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_blank_server_url_to_default() {
        assert_eq!(normalized_or_default("  ".to_owned()), DEFAULT_SERVER_URL);
    }

    #[test]
    fn converts_port_to_loopback_url() {
        assert_eq!(url_for_port(3017), "http://127.0.0.1:3017");
    }
}
