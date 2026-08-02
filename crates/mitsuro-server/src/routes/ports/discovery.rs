use std::collections::{BTreeSet, HashMap, HashSet};

use axum::{extract::State, Json};
use serde::Serialize;

use mitsuro_core::ports::{discover_listening_tcp_ports, TcpListenerInfo};
use mitsuro_core::process::ProcessInfo;

use super::probe::{probe_ports_previewability, ProbeStatus};
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::AppState;

use super::super::preview_settings::{load_preview_settings, PreviewSettings};

#[derive(Debug, Clone, Serialize)]
struct PortEntry {
    port: u16,
    name: String,
    description: Option<String>,
    command: Option<String>,
    pid: Option<u32>,
    source: String,
    active: bool,
    pinned: bool,
    is_http_like: bool,
    is_previewable_http: bool,
    probe_status: ProbeStatus,
    last_probe_ms: Option<u32>,
    preview_path: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct PortListResponse {
    ports: Vec<PortEntry>,
    settings: PreviewSettings,
    discovery_error: Option<String>,
}

#[derive(Debug)]
struct ProcessSearchEntry<'a> {
    process: &'a ProcessInfo,
    command_lower: String,
}

pub(super) async fn list_ports(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<PortListResponse>, AppError> {
    let settings = load_preview_settings(&state, user.as_ref())?;
    if !settings.enabled {
        return Ok(Json(PortListResponse {
            ports: vec![],
            settings,
            discovery_error: None,
        }));
    }

    let (listeners, discovery_error) = match discover_listening_tcp_ports() {
        Ok(listeners) => (listeners, None),
        Err(err) => {
            tracing::warn!(
                "Port discovery failed, falling back to pinned only: {}",
                err
            );
            (
                vec![],
                Some("Port discovery failed. Showing pinned ports only.".to_string()),
            )
        }
    };
    let discovered_by_port: HashMap<u16, TcpListenerInfo> = listeners
        .into_iter()
        .map(|listener| (listener.port, listener))
        .collect();

    let tracked_processes = match user.as_ref().and_then(|u| u.0.user_id.as_deref()) {
        Some(user_id) => state.process_registry.list_for_user(user_id).await,
        None => state.process_registry.list().await,
    };
    let running_processes: Vec<ProcessInfo> = tracked_processes
        .into_iter()
        .filter(|process| process.is_running())
        .collect();
    let mut tracked_by_pid = HashMap::with_capacity(running_processes.len());
    let mut running_process_search = Vec::with_capacity(running_processes.len());
    for process in &running_processes {
        if let Some(pid) = process.pid {
            tracked_by_pid.insert(pid, process);
        }
        running_process_search.push(ProcessSearchEntry {
            process,
            command_lower: process.command.to_ascii_lowercase(),
        });
    }

    let blocked_ports: HashSet<u16> = settings.blocked_ports.iter().copied().collect();
    let hidden_ports: HashSet<u16> = settings.hidden_ports.iter().copied().collect();
    let pinned_ports: HashSet<u16> = settings.pinned_ports.iter().copied().collect();

    let mut candidate_ports: BTreeSet<u16> = discovered_by_port.keys().copied().collect();
    candidate_ports.extend(pinned_ports.iter().copied());

    let mut ports = Vec::with_capacity(candidate_ports.len());
    for port in candidate_ports {
        if port == state.server_port
            || blocked_ports.contains(&port)
            || hidden_ports.contains(&port)
        {
            continue;
        }

        let listener = discovered_by_port.get(&port);
        let pinned = pinned_ports.contains(&port);
        let active = listener.is_some();
        let source = match (active, pinned) {
            (true, true) => "discovered+pinned",
            (true, false) => "discovered",
            (false, true) => "pinned",
            (false, false) => "discovered",
        }
        .to_string();

        let process_hint =
            resolve_process_hint(port, listener, &tracked_by_pid, &running_process_search);
        let description = process_hint.and_then(|process| process.description.clone());
        let command = process_hint.map(|process| process.command.clone());
        let pid = process_hint.and_then(|process| process.pid);
        let name = infer_display_name(port, description.as_deref(), command.as_deref());
        let is_http_like = infer_http_like(
            port,
            name.as_str(),
            description.as_deref(),
            command.as_deref(),
        );

        if settings.show_only_http_like && !is_http_like && !pinned {
            continue;
        }

        ports.push(PortEntry {
            port,
            name,
            description,
            command,
            pid,
            source,
            active,
            pinned,
            is_http_like,
            is_previewable_http: false,
            probe_status: ProbeStatus::ConnRefused,
            last_probe_ms: None,
            preview_path: format!("/api/ports/{}/proxy", port),
        });
    }

    let active_ports: Vec<u16> = ports
        .iter()
        .filter(|entry| entry.active)
        .map(|entry| entry.port)
        .collect();
    let probe_results = probe_ports_previewability(&active_ports, settings.probe_timeout_ms).await;
    for entry in &mut ports {
        if let Some(probe) = probe_results.get(&entry.port) {
            entry.is_previewable_http = probe.is_previewable_http;
            entry.probe_status = probe.status;
            entry.last_probe_ms = Some(probe.duration_ms);
        }
    }

    ports.sort_by(|left, right| {
        right
            .pinned
            .cmp(&left.pinned)
            .then_with(|| right.active.cmp(&left.active))
            .then_with(|| left.port.cmp(&right.port))
    });

    Ok(Json(PortListResponse {
        ports,
        settings,
        discovery_error,
    }))
}

fn resolve_process_hint<'a>(
    port: u16,
    listener: Option<&TcpListenerInfo>,
    tracked_by_pid: &'a HashMap<u32, &'a ProcessInfo>,
    running_processes: &'a [ProcessSearchEntry<'a>],
) -> Option<&'a ProcessInfo> {
    if let Some(listener) = listener {
        for pid in &listener.pids {
            if let Some(process) = tracked_by_pid.get(pid) {
                return Some(process);
            }
        }
    }

    let needle_colon = format!(":{}", port);
    let needle_port_eq = format!("--port={}", port);
    let needle_port_sep = format!("--port {}", port);
    let needle_short = format!("-p {}", port);

    running_processes
        .iter()
        .find(|entry| {
            let command = entry.command_lower.as_str();
            command.contains(&needle_colon)
                || command.contains(&needle_port_eq)
                || command.contains(&needle_port_sep)
                || command.contains(&needle_short)
        })
        .map(|entry| entry.process)
}

fn infer_display_name(port: u16, description: Option<&str>, command: Option<&str>) -> String {
    if let Some(description) = description.filter(|value| !value.trim().is_empty()) {
        return description.to_string();
    }

    if let Some(command) = command {
        const DISPLAY_HINTS: [(&str, &str); 9] = [
            ("vite", "Vite Dev Server"),
            ("next", "Next.js Dev Server"),
            ("webpack", "Webpack Dev Server"),
            ("astro", "Astro Dev Server"),
            ("nuxt", "Nuxt Dev Server"),
            ("storybook", "Storybook"),
            ("uvicorn", "Python Web Server"),
            ("gunicorn", "Python Web Server"),
            ("http.server", "Python HTTP Server"),
        ];
        let command = command.to_ascii_lowercase();
        for (needle, label) in DISPLAY_HINTS {
            if command.contains(needle) {
                return label.to_string();
            }
        }
    }

    format!("Port {}", port)
}

fn infer_http_like(
    port: u16,
    name: &str,
    description: Option<&str>,
    command: Option<&str>,
) -> bool {
    const COMMON_HTTP_PORTS: [u16; 18] = [
        80, 3000, 3001, 3002, 4000, 4173, 4200, 4321, 5000, 5173, 5174, 5175, 6006, 8000, 8080,
        8081, 8787, 9000,
    ];
    if COMMON_HTTP_PORTS.contains(&port) {
        return true;
    }

    let name = name.to_ascii_lowercase();
    let description = description.unwrap_or_default().to_ascii_lowercase();
    let command = command.unwrap_or_default().to_ascii_lowercase();
    const HTTP_KEYWORDS: [&str; 13] = [
        "vite",
        "next",
        "webpack",
        "astro",
        "nuxt",
        "storybook",
        "serve",
        "http",
        "uvicorn",
        "gunicorn",
        "flask",
        "django",
        "rails",
    ];
    HTTP_KEYWORDS.iter().any(|keyword| {
        name.contains(keyword) || description.contains(keyword) || command.contains(keyword)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infer_http_like_prefers_common_dev_ports() {
        assert!(infer_http_like(5173, "Port 5173", None, None));
        assert!(infer_http_like(3000, "Port 3000", None, None));
        assert!(!infer_http_like(9922, "Port 9922", None, None));
    }

    #[test]
    fn infer_http_like_uses_command_and_description_keywords() {
        assert!(infer_http_like(
            9922,
            "Port 9922",
            Some("local vite frontend"),
            None,
        ));
        assert!(infer_http_like(
            9922,
            "Port 9922",
            None,
            Some("uvicorn app.main:app --port 9922"),
        ));
    }
}
