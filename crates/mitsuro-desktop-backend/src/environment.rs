//! Environment / computer-use protocol types (scaffold).
//!
//! Typed Codex app-server environment methods:
//! - `environment/info`
//! - `environment/status`
//! - `environment/add`
//! - `collaborationMode/list` (optional companion surface)
//!
//! There is **no** `environment/list` method in the app-server protocol. The
//! fixture exposes a demo catalog via [`fixture_demo_environments`] for UI
//! listing (local + remote stub). Native Computer Use (`cua_node`) is **not**
//! ported — this module is protocol scaffold only.
//!
//! Includes environment info, status, add, and collaboration-mode shapes.
//! `CollaborationModeList*.json`.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// environment/status · EnvironmentStatusKind
// ---------------------------------------------------------------------------

/// Current status observed without starting or recovering an environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum EnvironmentStatusKind {
    /// Local env, or remote exec-server answered over an existing connection.
    #[default]
    Ready,
    /// Configured but not connected / still starting.
    Pending,
    /// Prior failure observed; `error` may explain.
    Disconnected,
    /// Environment id not configured.
    Unknown,
}

impl EnvironmentStatusKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Pending => "pending",
            Self::Disconnected => "disconnected",
            Self::Unknown => "unknown",
        }
    }

    /// Short UI chip label.
    pub fn status_label(self) -> &'static str {
        match self {
            Self::Ready => "connected",
            Self::Pending => "pending",
            Self::Disconnected => "disconnected",
            Self::Unknown => "unknown",
        }
    }

    pub fn is_connected(self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// Local vs remote execution environment (UI classification; not a wire enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum EnvironmentKind {
    #[default]
    Local,
    Remote,
}

impl EnvironmentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
        }
    }
}

// ---------------------------------------------------------------------------
// environment/info
// ---------------------------------------------------------------------------

/// Params for `environment/info`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentInfoParams {
    pub environment_id: String,
}

impl EnvironmentInfoParams {
    pub fn new(environment_id: impl Into<String>) -> Self {
        Self {
            environment_id: environment_id.into(),
        }
    }
}

/// Shell metadata reported by an environment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentShellInfo {
    /// Stable shell name (`bash`, `zsh`, `powershell`, …).
    pub name: String,
    /// Target-native shell path or command name.
    pub path: String,
}

/// Response for `environment/info`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentInfoResponse {
    pub shell: EnvironmentShellInfo,
    /// Default working directory as a canonical file URI (or null).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

// ---------------------------------------------------------------------------
// environment/status
// ---------------------------------------------------------------------------

/// Params for `environment/status`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentStatusParams {
    pub environment_id: String,
}

impl EnvironmentStatusParams {
    pub fn new(environment_id: impl Into<String>) -> Self {
        Self {
            environment_id: environment_id.into(),
        }
    }
}

/// Response for `environment/status`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentStatusResponse {
    pub status: EnvironmentStatusKind,
    /// Human-readable detail for `disconnected` / `unknown`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// environment/add
// ---------------------------------------------------------------------------

/// Params for `environment/add` (register a remote exec-server).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentAddParams {
    pub environment_id: String,
    pub exec_server_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect_timeout_ms: Option<u64>,
}

impl EnvironmentAddParams {
    pub fn new(environment_id: impl Into<String>, exec_server_url: impl Into<String>) -> Self {
        Self {
            environment_id: environment_id.into(),
            exec_server_url: exec_server_url.into(),
            connect_timeout_ms: None,
        }
    }
}

/// Response for `environment/add` (empty object on success).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentAddResponse {}

// ---------------------------------------------------------------------------
// collaborationMode/list
// ---------------------------------------------------------------------------

/// Params for `collaborationMode/list` (empty object).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CollaborationModeListParams {}

/// Collaboration mode kind advertised by presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ModeKind {
    Plan,
    #[default]
    Default,
}

impl ModeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Default => "default",
        }
    }
}

/// EXPERIMENTAL collaboration mode preset metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CollaborationModeMask {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<ModeKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Wire field is `reasoning_effort` (snake on wire per schema).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "reasoningEffort"
    )]
    pub reasoning_effort: Option<String>,
}

/// Response for `collaborationMode/list`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CollaborationModeListResponse {
    pub data: Vec<CollaborationModeMask>,
}

// ---------------------------------------------------------------------------
// Demo catalog (UI helper — not a protocol list method)
// ---------------------------------------------------------------------------

/// One row in the fixture/UI environment catalog.
///
/// Protocol has no `environment/list`; this is the offline list surface.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentSummary {
    pub id: String,
    pub name: String,
    pub kind: EnvironmentKind,
    pub status: EnvironmentStatusKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exec_server_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<EnvironmentShellInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

impl EnvironmentSummary {
    pub fn status_label(&self) -> &'static str {
        self.status.status_label()
    }

    pub fn kind_label(&self) -> &'static str {
        self.kind.as_str()
    }

    pub fn is_connected(&self) -> bool {
        self.status.is_connected()
    }
}

/// Offline demo environments: local (ready) + remote stub (disconnected).
pub fn fixture_demo_environments() -> Vec<EnvironmentSummary> {
    vec![
        EnvironmentSummary {
            id: "local".into(),
            name: "Local machine".into(),
            kind: EnvironmentKind::Local,
            status: EnvironmentStatusKind::Ready,
            description: Some("Local environment · host shell".into()),
            exec_server_url: None,
            error: None,
            shell: Some(EnvironmentShellInfo {
                name: "bash".into(),
                path: "/bin/bash".into(),
            }),
            cwd: Some("file:///fixture-project".into()),
        },
        EnvironmentSummary {
            id: "remote-stub".into(),
            name: "Remote stub".into(),
            kind: EnvironmentKind::Remote,
            status: EnvironmentStatusKind::Disconnected,
            description: Some("Remote exec-server · not connected".into()),
            exec_server_url: Some("wss://fixture.example/exec-server".into()),
            error: Some("remote exec-server not reachable (fixture stub)".into()),
            shell: Some(EnvironmentShellInfo {
                name: "bash".into(),
                path: "/usr/bin/bash".into(),
            }),
            cwd: None,
        },
    ]
}

/// Resolve `environment/info` against the demo catalog (or added remotes).
pub fn fixture_environment_info(
    environment_id: &str,
    extras: &[EnvironmentSummary],
) -> Option<EnvironmentInfoResponse> {
    let catalog = fixture_demo_environments();
    let entry = catalog
        .iter()
        .chain(extras.iter())
        .find(|e| e.id == environment_id)?;
    let shell = entry.shell.clone().unwrap_or(EnvironmentShellInfo {
        name: "sh".into(),
        path: "/bin/sh".into(),
    });
    Some(EnvironmentInfoResponse {
        shell,
        cwd: entry.cwd.clone(),
    })
}

/// Resolve `environment/status` against the demo catalog (or added remotes).
pub fn fixture_environment_status(
    environment_id: &str,
    extras: &[EnvironmentSummary],
) -> EnvironmentStatusResponse {
    let catalog = fixture_demo_environments();
    if let Some(entry) = catalog
        .iter()
        .chain(extras.iter())
        .find(|e| e.id == environment_id)
    {
        return EnvironmentStatusResponse {
            status: entry.status,
            error: entry.error.clone(),
        };
    }
    EnvironmentStatusResponse {
        status: EnvironmentStatusKind::Unknown,
        error: Some(format!("environment id not configured: {environment_id}")),
    }
}

/// Offline demo collaboration mode presets.
pub fn fixture_demo_collaboration_modes() -> CollaborationModeListResponse {
    CollaborationModeListResponse {
        data: vec![
            CollaborationModeMask {
                name: "Default".into(),
                mode: Some(ModeKind::Default),
                model: Some("gpt-5.3-codex".into()),
                reasoning_effort: Some("medium".into()),
            },
            CollaborationModeMask {
                name: "Plan".into(),
                mode: Some(ModeKind::Plan),
                model: Some("gpt-5.3-codex".into()),
                reasoning_effort: Some("high".into()),
            },
            CollaborationModeMask {
                name: "Pair".into(),
                mode: Some(ModeKind::Default),
                model: None,
                reasoning_effort: Some("low".into()),
            },
        ],
    }
}

/// Build the session-local summary for a successfully registered remote environment.
///
/// Codex returns an empty object from `environment/add` and exposes no list method, so
/// the desktop retains the exact submitted identity and URL while status is probed.
pub fn registered_environment_summary(params: &EnvironmentAddParams) -> EnvironmentSummary {
    EnvironmentSummary {
        id: params.environment_id.clone(),
        name: format!("Remote · {}", params.environment_id),
        kind: EnvironmentKind::Remote,
        status: EnvironmentStatusKind::Pending,
        description: Some(format!(
            "Added via environment/add · {}",
            params.exec_server_url
        )),
        exec_server_url: Some(params.exec_server_url.clone()),
        error: None,
        shell: None,
        cwd: None,
    }
}

/// Backward-compatible fixture helper.
pub fn fixture_added_environment_summary(params: &EnvironmentAddParams) -> EnvironmentSummary {
    registered_environment_summary(params)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_demo_env_list_local_and_remote() {
        let envs = fixture_demo_environments();
        assert_eq!(envs.len(), 2);
        assert!(envs
            .iter()
            .any(|e| e.id == "local" && e.kind == EnvironmentKind::Local));
        assert!(envs
            .iter()
            .any(|e| e.id == "remote-stub" && e.kind == EnvironmentKind::Remote));
        let local = envs.iter().find(|e| e.id == "local").unwrap();
        assert_eq!(local.status, EnvironmentStatusKind::Ready);
        assert!(local.is_connected());
        let remote = envs.iter().find(|e| e.id == "remote-stub").unwrap();
        assert_eq!(remote.status, EnvironmentStatusKind::Disconnected);
        assert!(!remote.is_connected());
    }

    #[test]
    fn fixture_environment_info_and_status() {
        let info = fixture_environment_info("local", &[]).expect("local info");
        assert_eq!(info.shell.name, "bash");
        assert!(info
            .cwd
            .as_deref()
            .unwrap_or("")
            .contains("fixture-project"));

        let st = fixture_environment_status("local", &[]);
        assert_eq!(st.status, EnvironmentStatusKind::Ready);
        assert!(st.error.is_none());

        let st_remote = fixture_environment_status("remote-stub", &[]);
        assert_eq!(st_remote.status, EnvironmentStatusKind::Disconnected);
        assert!(st_remote.error.is_some());

        let unknown = fixture_environment_status("no-such-env", &[]);
        assert_eq!(unknown.status, EnvironmentStatusKind::Unknown);
        assert!(fixture_environment_info("no-such-env", &[]).is_none());
    }

    #[test]
    fn serialize_environment_status_camel_case() {
        let resp = EnvironmentStatusResponse {
            status: EnvironmentStatusKind::Ready,
            error: None,
        };
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["status"], "ready");

        let params = EnvironmentStatusParams::new("local");
        let pv = serde_json::to_value(&params).unwrap();
        assert_eq!(pv["environmentId"], "local");
    }

    #[test]
    fn fixture_collaboration_modes() {
        let modes = fixture_demo_collaboration_modes();
        assert!(
            (2..=4).contains(&modes.data.len()),
            "expected 2–4 modes, got {}",
            modes.data.len()
        );
        assert!(modes.data.iter().any(|m| m.name == "Default"));
        assert!(modes.data.iter().any(|m| m.mode == Some(ModeKind::Plan)));
        let raw = serde_json::to_value(&modes).unwrap();
        assert!(raw.get("data").and_then(|d| d.as_array()).is_some());
    }

    #[test]
    fn fixture_add_builds_pending_remote() {
        let p = EnvironmentAddParams::new("env-new", "wss://example/exec");
        let s = fixture_added_environment_summary(&p);
        assert_eq!(s.id, "env-new");
        assert_eq!(s.kind, EnvironmentKind::Remote);
        assert_eq!(s.status, EnvironmentStatusKind::Pending);
        let info = fixture_environment_info("env-new", std::slice::from_ref(&s));
        assert!(info.is_some());
        let st = fixture_environment_status("env-new", std::slice::from_ref(&s));
        assert_eq!(st.status, EnvironmentStatusKind::Pending);
    }
}
