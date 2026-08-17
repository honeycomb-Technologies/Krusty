//! Typed Codex app-server Guardian follow-up contracts.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Temporary Codex auto-review notification shape. The action remains a raw
/// value because the upstream schema is explicitly unstable; conversion to the
/// approval event validates every variant before exposing it to the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardianApprovalReviewNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub review_id: String,
    #[serde(default)]
    pub target_item_id: Option<String>,
    pub action: Value,
    pub review: GuardianApprovalReview,
    #[serde(default)]
    pub decision_source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardianApprovalReview {
    pub status: GuardianApprovalReviewStatus,
    #[serde(default)]
    pub risk_level: Option<String>,
    #[serde(default)]
    pub user_authorization: Option<String>,
    #[serde(default)]
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GuardianApprovalReviewStatus {
    InProgress,
    Approved,
    Denied,
    TimedOut,
    Aborted,
}

impl GuardianApprovalReviewNotification {
    /// Build the exact snake-case GuardianAssessmentEvent used by the reference
    /// desktop for `thread/approveGuardianDeniedAction`.
    pub fn denied_assessment_event(&self) -> Option<Value> {
        if self.review.status != GuardianApprovalReviewStatus::Denied {
            return None;
        }
        Some(serde_json::json!({
            "id": self.review_id,
            "target_item_id": self.target_item_id,
            "turn_id": self.turn_id,
            "status": "denied",
            "risk_level": self.review.risk_level,
            "user_authorization": self.review.user_authorization,
            "rationale": self.review.rationale,
            "decision_source": self.decision_source,
            "action": assessment_action(&self.action)?,
        }))
    }

    pub fn action_title(&self) -> Option<String> {
        let action_type = self.action.get("type")?.as_str()?;
        match action_type {
            "command" => text(&self.action, "command"),
            "execve" => {
                let mut parts = vec![text(&self.action, "program")?];
                parts.extend(
                    self.action
                        .get("argv")?
                        .as_array()?
                        .iter()
                        .map(|value| value.as_str().map(str::to_owned))
                        .collect::<Option<Vec<_>>>()?,
                );
                Some(parts.join(" "))
            }
            "applyPatch" => {
                let count = self.action.get("files")?.as_array()?.len();
                Some(format!(
                    "Apply patch · {count} file{}",
                    if count == 1 { "" } else { "s" }
                ))
            }
            "networkAccess" => text(&self.action, "target").or_else(|| {
                Some(format!(
                    "{}:{}",
                    text(&self.action, "host")?,
                    self.action.get("port")?.as_u64()?
                ))
            }),
            "mcpToolCall" => self
                .action
                .get("toolTitle")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    Some(format!(
                        "{} · {}",
                        text(&self.action, "server")?,
                        text(&self.action, "toolName")?
                    ))
                }),
            "requestPermissions" => self
                .action
                .get("reason")
                .and_then(Value::as_str)
                .filter(|reason| !reason.is_empty())
                .map(str::to_owned)
                .or_else(|| Some("Additional permissions".to_owned())),
            _ => None,
        }
    }
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_owned)
}

fn assessment_action(action: &Value) -> Option<Value> {
    let action_type = action.get("type")?.as_str()?;
    match action_type {
        "command" => Some(serde_json::json!({
            "type": "command",
            "source": match text(action, "source")?.as_str() {
                "shell" => "shell",
                "unifiedExec" => "unified_exec",
                _ => return None,
            },
            "command": text(action, "command")?,
            "cwd": text(action, "cwd")?,
        })),
        "execve" => Some(serde_json::json!({
            "type": "execve",
            "source": match text(action, "source")?.as_str() {
                "shell" => "shell",
                "unifiedExec" => "unified_exec",
                _ => return None,
            },
            "program": text(action, "program")?,
            "argv": action.get("argv")?.as_array()?.iter()
                .map(|value| value.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()?,
            "cwd": text(action, "cwd")?,
        })),
        "applyPatch" => Some(serde_json::json!({
            "type": "apply_patch",
            "cwd": text(action, "cwd")?,
            "files": action.get("files")?.as_array()?.iter()
                .map(|value| value.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()?,
        })),
        "networkAccess" => Some(serde_json::json!({
            "type": "network_access",
            "target": text(action, "target")?,
            "host": text(action, "host")?,
            "protocol": match text(action, "protocol")?.as_str() {
                "http" => "http",
                "https" => "https",
                "socks5Tcp" => "socks5_tcp",
                "socks5Udp" => "socks5_udp",
                _ => return None,
            },
            "port": action.get("port")?.as_u64()?,
        })),
        "mcpToolCall" => Some(serde_json::json!({
            "type": "mcp_tool_call",
            "server": text(action, "server")?,
            "tool_name": text(action, "toolName")?,
            "connector_id": action.get("connectorId").cloned().unwrap_or(Value::Null),
            "connector_name": action.get("connectorName").cloned().unwrap_or(Value::Null),
            "tool_title": action.get("toolTitle").cloned().unwrap_or(Value::Null),
        })),
        "requestPermissions" => {
            let permissions = action.get("permissions")?.as_object()?;
            Some(serde_json::json!({
                "type": "request_permissions",
                "reason": action.get("reason").cloned().unwrap_or(Value::Null),
                "permissions": {
                    "network": permissions.get("network").cloned().unwrap_or(Value::Null),
                    "file_system": permissions.get("fileSystem").cloned().unwrap_or(Value::Null),
                },
            }))
        }
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadApproveGuardianDeniedActionParams {
    pub thread_id: String,
    /// Serialized `GuardianAssessmentEvent` supplied by the server.
    pub event: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadApproveGuardianDeniedActionResponse {}

#[cfg(test)]
mod tests {
    use super::*;

    fn denied_notification(action: Value) -> GuardianApprovalReviewNotification {
        GuardianApprovalReviewNotification {
            thread_id: "thread-1".to_owned(),
            turn_id: "turn-1".to_owned(),
            review_id: "review-1".to_owned(),
            target_item_id: Some("item-1".to_owned()),
            action,
            review: GuardianApprovalReview {
                status: GuardianApprovalReviewStatus::Denied,
                risk_level: Some("high".to_owned()),
                user_authorization: Some("medium".to_owned()),
                rationale: Some("Denied by policy".to_owned()),
            },
            decision_source: Some("agent".to_owned()),
        }
    }

    #[test]
    fn guardian_params_keep_the_opaque_assessment_event() {
        let params = ThreadApproveGuardianDeniedActionParams {
            thread_id: "thread-1".to_owned(),
            event: serde_json::json!({"action": "network", "reason": "policy"}),
        };
        assert_eq!(
            serde_json::to_value(params).unwrap(),
            serde_json::json!({
                "threadId": "thread-1",
                "event": {"action": "network", "reason": "policy"}
            })
        );
    }

    #[test]
    fn denied_notification_builds_the_reference_assessment_event() {
        let notification: GuardianApprovalReviewNotification =
            serde_json::from_value(serde_json::json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "reviewId": "review-1",
                "targetItemId": "item-1",
                "decisionSource": "agent",
                "action": {
                    "type": "networkAccess",
                    "target": "api.example.com",
                    "host": "api.example.com",
                    "protocol": "socks5Tcp",
                    "port": 443
                },
                "review": {
                    "status": "denied",
                    "riskLevel": "high",
                    "userAuthorization": "medium",
                    "rationale": "Network target was not authorized"
                },
                "startedAtMs": 10,
                "completedAtMs": 20
            }))
            .unwrap();
        assert_eq!(
            notification.action_title().as_deref(),
            Some("api.example.com")
        );
        assert_eq!(
            notification.denied_assessment_event().unwrap(),
            serde_json::json!({
                "id": "review-1",
                "target_item_id": "item-1",
                "turn_id": "turn-1",
                "status": "denied",
                "risk_level": "high",
                "user_authorization": "medium",
                "rationale": "Network target was not authorized",
                "decision_source": "agent",
                "action": {
                    "type": "network_access",
                    "target": "api.example.com",
                    "host": "api.example.com",
                    "protocol": "socks5_tcp",
                    "port": 443
                }
            })
        );
    }

    #[test]
    fn denied_notification_converts_every_reference_action_variant() {
        let cases = [
            (
                serde_json::json!({
                    "type": "command",
                    "source": "unifiedExec",
                    "command": "cargo test",
                    "cwd": "/workspace"
                }),
                serde_json::json!({
                    "type": "command",
                    "source": "unified_exec",
                    "command": "cargo test",
                    "cwd": "/workspace"
                }),
            ),
            (
                serde_json::json!({
                    "type": "execve",
                    "source": "shell",
                    "program": "cargo",
                    "argv": ["test", "-p", "desktop"],
                    "cwd": "/workspace"
                }),
                serde_json::json!({
                    "type": "execve",
                    "source": "shell",
                    "program": "cargo",
                    "argv": ["test", "-p", "desktop"],
                    "cwd": "/workspace"
                }),
            ),
            (
                serde_json::json!({
                    "type": "applyPatch",
                    "cwd": "/workspace",
                    "files": ["src/app.rs", "Cargo.toml"]
                }),
                serde_json::json!({
                    "type": "apply_patch",
                    "cwd": "/workspace",
                    "files": ["src/app.rs", "Cargo.toml"]
                }),
            ),
            (
                serde_json::json!({
                    "type": "mcpToolCall",
                    "server": "docs",
                    "toolName": "search",
                    "connectorId": "connector-1",
                    "connectorName": "Documentation",
                    "toolTitle": "Search docs"
                }),
                serde_json::json!({
                    "type": "mcp_tool_call",
                    "server": "docs",
                    "tool_name": "search",
                    "connector_id": "connector-1",
                    "connector_name": "Documentation",
                    "tool_title": "Search docs"
                }),
            ),
            (
                serde_json::json!({
                    "type": "requestPermissions",
                    "reason": "Read generated output",
                    "permissions": {
                        "network": {"enabled": false},
                        "fileSystem": {"read": ["/tmp/output"]}
                    }
                }),
                serde_json::json!({
                    "type": "request_permissions",
                    "reason": "Read generated output",
                    "permissions": {
                        "network": {"enabled": false},
                        "file_system": {"read": ["/tmp/output"]}
                    }
                }),
            ),
        ];

        for (action, expected_action) in cases {
            let event = denied_notification(action)
                .denied_assessment_event()
                .unwrap();
            assert_eq!(event.get("action"), Some(&expected_action));
        }
    }

    #[test]
    fn guardian_conversion_fails_closed_for_non_denials_and_malformed_actions() {
        let mut approved = denied_notification(serde_json::json!({
            "type": "command",
            "source": "shell",
            "command": "pwd",
            "cwd": "/workspace"
        }));
        approved.review.status = GuardianApprovalReviewStatus::Approved;
        assert_eq!(approved.denied_assessment_event(), None);

        let malformed = [
            serde_json::json!({"type": "unknown"}),
            serde_json::json!({
                "type": "command",
                "source": "unexpected",
                "command": "pwd",
                "cwd": "/workspace"
            }),
            serde_json::json!({
                "type": "applyPatch",
                "cwd": "/workspace",
                "files": [1]
            }),
            serde_json::json!({
                "type": "networkAccess",
                "target": "example.com",
                "host": "example.com",
                "protocol": "ftp",
                "port": 21
            }),
        ];
        for action in malformed {
            assert_eq!(denied_notification(action).denied_assessment_event(), None);
        }
    }
}
