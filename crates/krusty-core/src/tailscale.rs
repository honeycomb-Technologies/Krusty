//! Tailscale integration for network access
//!
//! Detects Tailscale, resolves device URLs, and manages `tailscale serve`
//! for exposing the Krusty server with automatic HTTPS on the tailnet.
//!
//! Uses HTTPS port 8443 to avoid clashing with other services that may
//! already be using the default port 443 on the same machine.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::process::Command;

/// Dedicated HTTPS port for Krusty on Tailscale.
/// Tailscale supports 443, 8443, and 10000. We use 8443 to avoid
/// clashing with other services on the default 443.
const TAILSCALE_HTTPS_PORT: u16 = 8443;

#[derive(Debug, Clone)]
pub struct TailscaleInfo {
    pub dns_name: String,
    pub tailnet_name: String,
    pub online: bool,
}

/// Outcome of attempting to configure Tailscale serve for Krusty.
#[derive(Debug, Clone)]
pub enum TailscaleServeSetup {
    NotInstalled,
    Offline,
    Configured { url: String },
    PermissionDenied { url: String, detail: String },
    Failed { detail: String },
}

#[derive(Deserialize)]
struct TailscaleStatus {
    #[serde(rename = "Self")]
    self_node: Option<SelfNode>,
    #[serde(rename = "CurrentTailnet")]
    current_tailnet: Option<TailnetInfo>,
}

#[derive(Deserialize)]
struct SelfNode {
    #[serde(rename = "DNSName")]
    dns_name: String,
    #[serde(rename = "Online")]
    online: bool,
}

#[derive(Deserialize)]
struct TailnetInfo {
    #[serde(rename = "Name")]
    name: String,
}

/// Check if the `tailscale` CLI is available on PATH.
pub fn is_installed() -> bool {
    which::which("tailscale").is_ok()
}

/// Get current Tailscale device info by running `tailscale status --json`.
pub fn device_info() -> Result<TailscaleInfo> {
    let output = Command::new("tailscale")
        .args(["status", "--json"])
        .output()
        .context("Failed to run `tailscale status --json`")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("tailscale status failed: {}", stderr.trim());
    }

    let status: TailscaleStatus =
        serde_json::from_slice(&output.stdout).context("Failed to parse tailscale status JSON")?;

    let self_node = status
        .self_node
        .context("No self node in tailscale status")?;

    let tailnet = status
        .current_tailnet
        .context("No tailnet info in tailscale status")?;

    // DNSName comes with trailing dot, strip it
    let dns_name = self_node.dns_name.trim_end_matches('.').to_string();

    Ok(TailscaleInfo {
        dns_name,
        tailnet_name: tailnet.name,
        online: self_node.online,
    })
}

/// Get the HTTPS URL for Krusty on the tailnet (port 8443).
pub fn device_url(_port: u16) -> Result<String> {
    let info = device_info()?;
    if !info.online {
        anyhow::bail!("Tailscale is installed but this device is offline");
    }
    Ok(format!(
        "https://{}:{}",
        info.dns_name, TAILSCALE_HTTPS_PORT
    ))
}

/// Run `tailscale serve --bg --https 8443 <port>` to expose on a dedicated HTTPS port.
///
/// Uses port 8443 so Krusty doesn't clash with other services on port 443.
pub fn serve(port: u16) -> Result<()> {
    let output = Command::new("tailscale")
        .args([
            "serve",
            "--bg",
            "--https",
            &TAILSCALE_HTTPS_PORT.to_string(),
            &port.to_string(),
        ])
        .output()
        .context("Failed to run `tailscale serve`")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("tailscale serve failed: {}", stderr.trim());
    }

    tracing::info!(
        "Tailscale serve configured: HTTPS:{} → localhost:{}",
        TAILSCALE_HTTPS_PORT,
        port
    );
    Ok(())
}

/// Stop `tailscale serve` for the krusty HTTPS port.
pub fn serve_stop(_port: u16) -> Result<()> {
    let output = Command::new("tailscale")
        .args(["serve", "--https", &TAILSCALE_HTTPS_PORT.to_string(), "off"])
        .output()
        .context("Failed to run `tailscale serve off`")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!("tailscale serve off failed: {}", stderr.trim());
    }

    Ok(())
}

/// Full Tailscale setup: detect, expose on port 8443, return URL.
pub fn setup_tailscale_serve(port: u16) -> TailscaleServeSetup {
    if !is_installed() {
        return TailscaleServeSetup::NotInstalled;
    }

    match device_info() {
        Ok(info) if !info.online => TailscaleServeSetup::Offline,
        Ok(info) => {
            let url = format!("https://{}:{}", info.dns_name, TAILSCALE_HTTPS_PORT);
            match serve(port) {
                Ok(_) => TailscaleServeSetup::Configured { url },
                Err(e) => {
                    let detail = e.to_string();
                    if is_permission_denied(&detail) {
                        TailscaleServeSetup::PermissionDenied { url, detail }
                    } else {
                        TailscaleServeSetup::Failed { detail }
                    }
                }
            }
        }
        Err(e) => TailscaleServeSetup::Failed {
            detail: format!("Tailscale detection failed: {}", e),
        },
    }
}

pub(crate) fn is_permission_denied(detail: &str) -> bool {
    let lower = detail.to_ascii_lowercase();
    lower.contains("access denied")
        || lower.contains("serve config denied")
        || lower.contains("--operator")
        || lower.contains("sudo tailscale serve")
        || lower.contains("must be root, or be an operator")
        || lower.contains("401 unauthorized")
}

pub(crate) fn command_contains_serve(command: &str) -> bool {
    let Ok(tokens) = shell_words::split(&command.to_ascii_lowercase()) else {
        return false;
    };

    tokens.iter().enumerate().any(|(index, token)| {
        std::path::Path::new(token)
            .file_name()
            .and_then(|name| name.to_str())
            == Some("tailscale")
            && tokens.get(index + 1).map(String::as_str) == Some("serve")
    })
}

#[cfg(test)]
mod tests {
    use super::{command_contains_serve, is_permission_denied};

    #[test]
    fn detects_tailscale_permission_denied_messages() {
        assert!(is_permission_denied(
            "tailscale serve failed: Access denied: serve config denied"
        ));
        assert!(is_permission_denied(
            "use 'sudo tailscale set --operator=$USER' once"
        ));
        assert!(is_permission_denied(
            "sending serve config: 401 Unauthorized: must be root, or be an operator and able to run 'sudo tailscale' to serve a path"
        ));
        assert!(!is_permission_denied(
            "tailscale serve failed: connection timeout"
        ));
    }

    #[test]
    fn detects_tailscale_serve_in_compound_commands() {
        assert!(command_contains_serve(
            "echo before; tailscale serve --bg --https=9443 http://127.0.0.1:5180; echo after"
        ));
        assert!(command_contains_serve("/usr/bin/tailscale serve status"));
        assert!(!command_contains_serve("echo 'tailscale serve --bg 3000'"));
        assert!(!command_contains_serve("tailscale status"));
    }
}
