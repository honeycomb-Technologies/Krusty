//! Typed Codex app-server Remote Control contracts.
//!
//! These shapes mirror the generated Codex v2 schema. Remote Control lets
//! authorized ChatGPT clients discover and control this Codex installation;
//! it is distinct from the Computer Use tool/plugin surface.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteControlConnectionStatus {
    Disabled,
    Connecting,
    Connected,
    Errored,
}

impl RemoteControlConnectionStatus {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Disabled => "Disabled",
            Self::Connecting => "Connecting",
            Self::Connected => "Connected",
            Self::Errored => "Connection failed",
        }
    }

    pub const fn is_enabled(self) -> bool {
        matches!(self, Self::Connecting | Self::Connected)
    }
}

/// Current connection state and identity exposed by the local app-server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteControlStatusReadResponse {
    pub status: RemoteControlConnectionStatus,
    pub server_name: String,
    pub installation_id: String,
    pub environment_id: Option<String>,
}

/// `remoteControl/status/changed` uses the same generated wire shape.
pub type RemoteControlStatusChangedNotification = RemoteControlStatusReadResponse;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteControlEnableParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ephemeral: Option<bool>,
}

pub type RemoteControlEnableResponse = RemoteControlStatusReadResponse;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteControlDisableParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ephemeral: Option<bool>,
}

pub type RemoteControlDisableResponse = RemoteControlStatusReadResponse;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteControlPairingStartParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manual_code: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteControlPairingStartResponse {
    pub pairing_code: String,
    pub manual_pairing_code: Option<String>,
    pub environment_id: String,
    /// Unix timestamp returned by Codex. The generated TypeScript contract is `bigint`.
    pub expires_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteControlPairingStatusParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pairing_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manual_pairing_code: Option<String>,
}

impl RemoteControlPairingStatusParams {
    pub fn from_pairing(pairing: &RemoteControlPairingStartResponse) -> Self {
        Self {
            pairing_code: Some(pairing.pairing_code.clone()),
            manual_pairing_code: pairing.manual_pairing_code.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteControlPairingStatusResponse {
    pub claimed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteControlClient {
    pub client_id: String,
    pub display_name: Option<String>,
    pub device_type: Option<String>,
    pub platform: Option<String>,
    pub os_version: Option<String>,
    pub device_model: Option<String>,
    pub app_version: Option<String>,
    /// Unix timestamp returned by Codex. The generated TypeScript contract is `bigint`.
    pub last_seen_at: Option<i64>,
}

impl RemoteControlClient {
    pub fn display_label(&self) -> &str {
        self.display_name
            .as_deref()
            .or(self.device_model.as_deref())
            .or(self.device_type.as_deref())
            .unwrap_or("Authorized device")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteControlClientsListOrder {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteControlClientsListParams {
    pub environment_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<RemoteControlClientsListOrder>,
}

impl RemoteControlClientsListParams {
    pub fn newest_first(environment_id: impl Into<String>) -> Self {
        Self {
            environment_id: environment_id.into(),
            cursor: None,
            limit: Some(100),
            order: Some(RemoteControlClientsListOrder::Desc),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteControlClientsListResponse {
    pub data: Vec<RemoteControlClient>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteControlClientsRevokeParams {
    pub environment_id: String,
    pub client_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteControlClientsRevokeResponse {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_remote_control_shapes_round_trip() {
        let status: RemoteControlStatusReadResponse = serde_json::from_value(serde_json::json!({
            "status": "connected",
            "serverName": "Jacob's PC",
            "installationId": "install-1",
            "environmentId": "env-1"
        }))
        .unwrap();
        assert_eq!(status.status, RemoteControlConnectionStatus::Connected);
        assert!(status.status.is_enabled());

        let client: RemoteControlClient = serde_json::from_value(serde_json::json!({
            "clientId": "phone-1",
            "displayName": "Jacob's phone",
            "deviceType": "phone",
            "platform": "ios",
            "osVersion": "26.0",
            "deviceModel": "iPhone",
            "appVersion": "1.0",
            "lastSeenAt": 1786320000
        }))
        .unwrap();
        assert_eq!(client.display_label(), "Jacob's phone");

        assert_eq!(
            serde_json::to_value(RemoteControlEnableParams::default()).unwrap(),
            serde_json::json!({})
        );
        assert_eq!(
            serde_json::to_value(RemoteControlPairingStartParams {
                manual_code: Some(true),
            })
            .unwrap(),
            serde_json::json!({"manualCode": true})
        );
    }
}
