//! Server→client approval requests (exec / patch) and client responses.
//!
//! Matches Codex app-server `ServerRequest` methods:
//! - Legacy: `execCommandApproval`, `applyPatchApproval`
//! - Turn-start: `item/commandExecution/requestApproval`, `item/fileChange/requestApproval`
//! - Additional sandbox access: `item/permissions/requestApproval`
//!
//! Shapes are modeled from the Codex app-server approval protocol.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::protocol::JsonRpcId;

// ---------------------------------------------------------------------------
// Methods
// ---------------------------------------------------------------------------

/// Server request methods that require a user approval decision.
pub const EXEC_COMMAND_APPROVAL: &str = "execCommandApproval";
pub const APPLY_PATCH_APPROVAL: &str = "applyPatchApproval";
pub const ITEM_COMMAND_EXECUTION_REQUEST_APPROVAL: &str = "item/commandExecution/requestApproval";
pub const ITEM_FILE_CHANGE_REQUEST_APPROVAL: &str = "item/fileChange/requestApproval";
pub const ITEM_PERMISSIONS_REQUEST_APPROVAL: &str = "item/permissions/requestApproval";

/// Returns true when `method` is an approval server-request the client must answer.
pub fn is_approval_method(method: &str) -> bool {
    matches!(
        method,
        EXEC_COMMAND_APPROVAL
            | APPLY_PATCH_APPROVAL
            | ITEM_COMMAND_EXECUTION_REQUEST_APPROVAL
            | ITEM_FILE_CHANGE_REQUEST_APPROVAL
            | ITEM_PERMISSIONS_REQUEST_APPROVAL
    )
}

// ---------------------------------------------------------------------------
// Kind / pending surface
// ---------------------------------------------------------------------------

/// Coarse approval family for UI + response shaping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApprovalKind {
    /// Legacy `execCommandApproval` (ReviewDecision).
    ExecCommand,
    /// Legacy `applyPatchApproval` (ReviewDecision).
    ApplyPatch,
    /// `item/commandExecution/requestApproval` (CommandExecutionApprovalDecision).
    CommandExecution,
    /// `item/fileChange/requestApproval` (FileChangeApprovalDecision).
    FileChange,
    /// `item/permissions/requestApproval` (exact requested permission profile).
    Permissions,
}

impl ApprovalKind {
    pub fn from_method(method: &str) -> Option<Self> {
        match method {
            EXEC_COMMAND_APPROVAL => Some(Self::ExecCommand),
            APPLY_PATCH_APPROVAL => Some(Self::ApplyPatch),
            ITEM_COMMAND_EXECUTION_REQUEST_APPROVAL => Some(Self::CommandExecution),
            ITEM_FILE_CHANGE_REQUEST_APPROVAL => Some(Self::FileChange),
            ITEM_PERMISSIONS_REQUEST_APPROVAL => Some(Self::Permissions),
            _ => None,
        }
    }

    pub fn method(self) -> &'static str {
        match self {
            Self::ExecCommand => EXEC_COMMAND_APPROVAL,
            Self::ApplyPatch => APPLY_PATCH_APPROVAL,
            Self::CommandExecution => ITEM_COMMAND_EXECUTION_REQUEST_APPROVAL,
            Self::FileChange => ITEM_FILE_CHANGE_REQUEST_APPROVAL,
            Self::Permissions => ITEM_PERMISSIONS_REQUEST_APPROVAL,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::ExecCommand | Self::CommandExecution => "Approve command",
            Self::ApplyPatch | Self::FileChange => "Approve file changes",
            Self::Permissions => "Approve additional permissions",
        }
    }

    /// True when the response uses legacy [`ReviewDecision`] wire values.
    pub fn uses_review_decision(self) -> bool {
        matches!(self, Self::ExecCommand | Self::ApplyPatch)
    }
}

/// Params for `item/permissions/requestApproval`.
///
/// `permissions` stays as JSON because the protocol's filesystem/network
/// profile is intentionally extensible. Approval echoes this exact server
/// request; the desktop never invents broader permissions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionsRequestApprovalParams {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub started_at_ms: i64,
    #[serde(default)]
    pub cwd: Option<String>,
    pub permissions: Value,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub environment_id: Option<String>,
}

/// Normalized pending approval for UI + response correlation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingApproval {
    /// JSON-RPC request id the client must answer with.
    pub request_id: JsonRpcId,
    pub method: String,
    pub kind: ApprovalKind,
    /// Short title ("Approve command" / "Approve file changes").
    pub title: String,
    /// One-line summary (command string or file list).
    pub summary: String,
    /// Extra context: cwd, reason, diff snippet, etc.
    pub detail: String,
    /// Optional thread binding when present on params.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Optional turn binding when present on params.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// Raw params object from the server request.
    pub raw_params: Value,
}

// ---------------------------------------------------------------------------
// Legacy params (execCommandApproval / applyPatchApproval)
// ---------------------------------------------------------------------------

/// Params for `execCommandApproval` (legacy SendUserTurn path).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecCommandApprovalParams {
    pub conversation_id: String,
    pub call_id: String,
    #[serde(default)]
    pub approval_id: Option<String>,
    pub command: Vec<String>,
    pub cwd: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub parsed_cmd: Vec<Value>,
}

/// A single file change entry inside `applyPatchApproval.fileChanges`.
///
/// Schema uses `type` discriminators (`add`/`delete`/`update`) with
/// `unified_diff` / `move_path` in snake_case (see ApplyPatchApprovalParams).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FileChange {
    #[serde(rename = "add")]
    Add { content: String },
    #[serde(rename = "delete")]
    Delete { content: String },
    #[serde(rename = "update")]
    Update {
        unified_diff: String,
        #[serde(default)]
        move_path: Option<String>,
    },
}

/// Params for `applyPatchApproval` (legacy).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyPatchApprovalParams {
    pub conversation_id: String,
    pub call_id: String,
    /// path → change
    #[serde(default)]
    pub file_changes: std::collections::BTreeMap<String, FileChange>,
    #[serde(default)]
    pub grant_root: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

// ---------------------------------------------------------------------------
// V2 turn/start params (subset used for display + response)
// ---------------------------------------------------------------------------

/// Params for `item/commandExecution/requestApproval`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandExecutionRequestApprovalParams {
    pub item_id: String,
    pub thread_id: String,
    pub turn_id: String,
    pub started_at_ms: i64,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub approval_id: Option<String>,
    #[serde(default)]
    pub proposed_execpolicy_amendment: Option<Vec<String>>,
}

/// Params for `item/fileChange/requestApproval`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChangeRequestApprovalParams {
    pub item_id: String,
    pub thread_id: String,
    pub turn_id: String,
    pub started_at_ms: i64,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub grant_root: Option<String>,
}

// ---------------------------------------------------------------------------
// ReviewDecision (legacy response)
// ---------------------------------------------------------------------------

/// User decision for legacy `execCommandApproval` / `applyPatchApproval`.
///
/// Wire shape is a string or a small object (see schema `ReviewDecision`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ReviewDecision {
    /// `"approved" | "approved_for_session" | "timed_out" | "abort"`
    Simple(ReviewDecisionSimple),
    ApprovedExecpolicyAmendment {
        approved_execpolicy_amendment: ProposedExecpolicyAmendmentBody,
    },
    NetworkPolicyAmendment {
        network_policy_amendment: NestedNetworkPolicyAmendment,
    },
    Denied {
        denied: DeniedBody,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecisionSimple {
    Approved,
    ApprovedForSession,
    TimedOut,
    Abort,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProposedExecpolicyAmendmentBody {
    pub proposed_execpolicy_amendment: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NestedNetworkPolicyAmendment {
    pub network_policy_amendment: NetworkPolicyAmendment,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetworkPolicyAmendment {
    pub action: NetworkPolicyRuleAction,
    pub host: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicyRuleAction {
    Allow,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeniedBody {
    pub rejection: String,
}

impl ReviewDecision {
    pub fn approved() -> Self {
        Self::Simple(ReviewDecisionSimple::Approved)
    }

    pub fn approved_for_session() -> Self {
        Self::Simple(ReviewDecisionSimple::ApprovedForSession)
    }

    pub fn abort() -> Self {
        Self::Simple(ReviewDecisionSimple::Abort)
    }

    pub fn denied(rejection: impl Into<String>) -> Self {
        Self::Denied {
            denied: DeniedBody {
                rejection: rejection.into(),
            },
        }
    }
}

/// Response body for legacy exec/patch approvals.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewDecisionResponse {
    pub decision: ReviewDecision,
}

// ---------------------------------------------------------------------------
// V2 command / file-change decisions
// ---------------------------------------------------------------------------

/// Decision for `item/commandExecution/requestApproval`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CommandExecutionApprovalDecision {
    Simple(CommandExecutionDecisionSimple),
    AcceptWithExecpolicyAmendment {
        #[serde(rename = "acceptWithExecpolicyAmendment")]
        accept_with_execpolicy_amendment: ExecpolicyAmendmentBody,
    },
    ApplyNetworkPolicyAmendment {
        #[serde(rename = "applyNetworkPolicyAmendment")]
        apply_network_policy_amendment: NestedNetworkPolicyAmendmentV2,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandExecutionDecisionSimple {
    #[serde(rename = "accept")]
    Accept,
    #[serde(rename = "acceptForSession")]
    AcceptForSession,
    #[serde(rename = "decline")]
    Decline,
    #[serde(rename = "cancel")]
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecpolicyAmendmentBody {
    pub execpolicy_amendment: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NestedNetworkPolicyAmendmentV2 {
    pub network_policy_amendment: NetworkPolicyAmendment,
}

impl CommandExecutionApprovalDecision {
    pub fn accept() -> Self {
        Self::Simple(CommandExecutionDecisionSimple::Accept)
    }

    pub fn decline() -> Self {
        Self::Simple(CommandExecutionDecisionSimple::Decline)
    }

    pub fn cancel() -> Self {
        Self::Simple(CommandExecutionDecisionSimple::Cancel)
    }

    pub fn accept_for_session() -> Self {
        Self::Simple(CommandExecutionDecisionSimple::AcceptForSession)
    }
}

/// Decision for `item/fileChange/requestApproval`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileChangeApprovalDecision {
    #[serde(rename = "accept")]
    Accept,
    #[serde(rename = "acceptForSession")]
    AcceptForSession,
    #[serde(rename = "decline")]
    Decline,
    #[serde(rename = "cancel")]
    Cancel,
}

/// Wrapper `{ "decision": ... }` matching all approval response schemas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalDecisionResponse<D> {
    pub decision: D,
}

// ---------------------------------------------------------------------------
// High-level approve / deny helpers
// ---------------------------------------------------------------------------

/// User-facing approve / reject choice (maps to protocol-specific decision).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalChoice {
    /// Allow the action and continue the turn.
    Approve,
    /// Deny the action but let the agent continue (when protocol supports it).
    Reject,
    /// Deny and interrupt / abort the turn where supported.
    Abort,
}

/// Build the JSON-RPC `result` object for an approval server request.
pub fn build_approval_result(kind: ApprovalKind, choice: ApprovalChoice) -> Value {
    match kind {
        ApprovalKind::ExecCommand | ApprovalKind::ApplyPatch => {
            let decision = match choice {
                ApprovalChoice::Approve => ReviewDecision::approved(),
                ApprovalChoice::Reject => ReviewDecision::denied("User rejected"),
                ApprovalChoice::Abort => ReviewDecision::abort(),
            };
            serde_json::to_value(ReviewDecisionResponse { decision })
                .expect("ReviewDecisionResponse serializes")
        }
        ApprovalKind::CommandExecution => {
            let decision = match choice {
                ApprovalChoice::Approve => CommandExecutionApprovalDecision::accept(),
                ApprovalChoice::Reject => CommandExecutionApprovalDecision::decline(),
                ApprovalChoice::Abort => CommandExecutionApprovalDecision::cancel(),
            };
            serde_json::to_value(ApprovalDecisionResponse { decision })
                .expect("CommandExecution response serializes")
        }
        ApprovalKind::FileChange => {
            let decision = match choice {
                ApprovalChoice::Approve => FileChangeApprovalDecision::Accept,
                ApprovalChoice::Reject => FileChangeApprovalDecision::Decline,
                ApprovalChoice::Abort => FileChangeApprovalDecision::Cancel,
            };
            serde_json::to_value(ApprovalDecisionResponse { decision })
                .expect("FileChange response serializes")
        }
        // This compatibility helper does not have the pending request payload.
        // The live response path uses `build_pending_approval_result` below.
        ApprovalKind::Permissions => json!({ "permissions": {}, "scope": "turn" }),
    }
}

/// Build a response using the pending request payload when the schema requires
/// it (notably permission grants).
pub fn build_pending_approval_result(pending: &PendingApproval, choice: ApprovalChoice) -> Value {
    if pending.kind != ApprovalKind::Permissions {
        return build_approval_result(pending.kind, choice);
    }

    let permissions = if matches!(choice, ApprovalChoice::Approve) {
        pending
            .raw_params
            .get("permissions")
            .cloned()
            .unwrap_or_else(|| json!({}))
    } else {
        json!({})
    };
    json!({ "permissions": permissions, "scope": "turn" })
}

/// Full JSON-RPC response line body (object, not yet newline-terminated).
pub fn build_approval_rpc_response(
    request_id: &JsonRpcId,
    kind: ApprovalKind,
    choice: ApprovalChoice,
) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "result": build_approval_result(kind, choice),
    })
}

// ---------------------------------------------------------------------------
// Parse server request → PendingApproval
// ---------------------------------------------------------------------------

/// Parse a classified server request into a [`PendingApproval`] when it is an
/// approval method. Returns `None` for non-approval server requests.
pub fn parse_approval_request(
    id: JsonRpcId,
    method: &str,
    params: Option<&Value>,
) -> Option<PendingApproval> {
    let kind = ApprovalKind::from_method(method)?;
    let p = params.cloned().unwrap_or(Value::Null);
    let (summary, detail, thread_id, turn_id) = match kind {
        ApprovalKind::ExecCommand => {
            let parsed: ExecCommandApprovalParams = serde_json::from_value(p.clone()).ok()?;
            let summary = if parsed.command.is_empty() {
                "(empty command)".into()
            } else {
                parsed.command.join(" ")
            };
            let mut detail = format!("cwd: {}", parsed.cwd);
            if let Some(r) = &parsed.reason {
                if !r.is_empty() {
                    detail.push('\n');
                    detail.push_str(r);
                }
            }
            (summary, detail, Some(parsed.conversation_id), None)
        }
        ApprovalKind::ApplyPatch => {
            let parsed: ApplyPatchApprovalParams = serde_json::from_value(p.clone()).ok()?;
            let paths: Vec<&str> = parsed.file_changes.keys().map(String::as_str).collect();
            let summary = if paths.is_empty() {
                "file changes".into()
            } else if paths.len() == 1 {
                paths[0].to_string()
            } else {
                format!("{} files", paths.len())
            };
            let mut detail = summarize_file_changes(&parsed.file_changes);
            if let Some(r) = &parsed.reason {
                if !r.is_empty() {
                    if !detail.is_empty() {
                        detail.push('\n');
                    }
                    detail.push_str(r);
                }
            }
            if let Some(root) = &parsed.grant_root {
                detail.push_str(&format!("\ngrantRoot: {root}"));
            }
            (summary, detail, Some(parsed.conversation_id), None)
        }
        ApprovalKind::CommandExecution => {
            // Prefer typed parse; fall back to best-effort fields.
            let (command, cwd, reason, thread_id, turn_id) = if let Ok(parsed) =
                serde_json::from_value::<CommandExecutionRequestApprovalParams>(p.clone())
            {
                (
                    parsed.command.unwrap_or_else(|| "(command)".into()),
                    parsed.cwd,
                    parsed.reason,
                    Some(parsed.thread_id),
                    Some(parsed.turn_id),
                )
            } else {
                (
                    p.get("command")
                        .and_then(|v| v.as_str())
                        .unwrap_or("(command)")
                        .to_string(),
                    p.get("cwd").and_then(|v| v.as_str()).map(str::to_string),
                    p.get("reason").and_then(|v| v.as_str()).map(str::to_string),
                    p.get("threadId")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    p.get("turnId").and_then(|v| v.as_str()).map(str::to_string),
                )
            };
            let mut detail = String::new();
            if let Some(c) = cwd {
                detail.push_str(&format!("cwd: {c}"));
            }
            if let Some(r) = reason {
                if !r.is_empty() {
                    if !detail.is_empty() {
                        detail.push('\n');
                    }
                    detail.push_str(&r);
                }
            }
            (command, detail, thread_id, turn_id)
        }
        ApprovalKind::FileChange => {
            let (reason, grant_root, thread_id, turn_id, item_id) = if let Ok(parsed) =
                serde_json::from_value::<FileChangeRequestApprovalParams>(p.clone())
            {
                (
                    parsed.reason,
                    parsed.grant_root,
                    Some(parsed.thread_id),
                    Some(parsed.turn_id),
                    Some(parsed.item_id),
                )
            } else {
                (
                    p.get("reason").and_then(|v| v.as_str()).map(str::to_string),
                    p.get("grantRoot")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    p.get("threadId")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    p.get("turnId").and_then(|v| v.as_str()).map(str::to_string),
                    p.get("itemId").and_then(|v| v.as_str()).map(str::to_string),
                )
            };
            let summary = item_id
                .map(|id| format!("file change · {id}"))
                .unwrap_or_else(|| "file change".into());
            let mut detail = String::new();
            if let Some(r) = reason {
                detail.push_str(&r);
            }
            if let Some(root) = grant_root {
                if !detail.is_empty() {
                    detail.push('\n');
                }
                detail.push_str(&format!("grantRoot: {root}"));
            }
            (summary, detail, thread_id, turn_id)
        }
        ApprovalKind::Permissions => {
            let parsed: PermissionsRequestApprovalParams =
                serde_json::from_value(p.clone()).ok()?;
            let mut requested = Vec::new();
            if parsed.permissions.get("fileSystem").is_some() {
                requested.push("filesystem");
            }
            if parsed.permissions.get("network").is_some() {
                requested.push("network");
            }
            let summary = if requested.is_empty() {
                "additional sandbox access".to_owned()
            } else {
                requested.join(" and ")
            };
            let mut details = Vec::new();
            if let Some(cwd) = parsed.cwd.filter(|value| !value.is_empty()) {
                details.push(format!("cwd: {cwd}"));
            }
            if let Some(reason) = parsed.reason.filter(|value| !value.is_empty()) {
                details.push(reason);
            }
            if let Some(environment_id) = parsed.environment_id.filter(|value| !value.is_empty()) {
                details.push(format!("environment: {environment_id}"));
            }
            (
                summary,
                details.join("\n"),
                Some(parsed.thread_id),
                Some(parsed.turn_id),
            )
        }
    };

    Some(PendingApproval {
        request_id: id,
        method: method.to_string(),
        kind,
        title: kind.title().to_string(),
        summary,
        detail,
        thread_id,
        turn_id,
        raw_params: p,
    })
}

fn summarize_file_changes(changes: &std::collections::BTreeMap<String, FileChange>) -> String {
    let mut lines = Vec::new();
    for (path, change) in changes.iter().take(6) {
        let label = match change {
            FileChange::Add { content } => {
                let n = content.lines().count();
                format!("+ {path} ({n} lines)")
            }
            FileChange::Delete { .. } => format!("− {path}"),
            FileChange::Update {
                unified_diff,
                move_path,
            } => {
                let n = unified_diff.lines().count();
                if let Some(dest) = move_path {
                    format!("~ {path} → {dest} ({n} diff lines)")
                } else {
                    format!("~ {path} ({n} diff lines)")
                }
            }
        };
        lines.push(label);
    }
    if changes.len() > 6 {
        lines.push(format!("… +{} more", changes.len() - 6));
    }
    lines.join("\n")
}

/// Map a server request (or pseudo-notification `serverRequest/{method}`) into
/// an optional pending approval.
pub fn pending_from_server_request_notification(
    method: &str,
    params: Option<&Value>,
) -> Option<PendingApproval> {
    // Live codex backend surfaces as method "serverRequest/<realMethod>" with
    // params { "id": ..., "params": ... }.
    let (real_method, id, inner) = if let Some(rest) = method.strip_prefix("serverRequest/") {
        let p = params?;
        let id = p.get("id").cloned()?;
        let id: JsonRpcId = serde_json::from_value(id).ok()?;
        let inner = p.get("params");
        (rest, id, inner)
    } else if is_approval_method(method) {
        // Bare server request already classified (fixture JSONL).
        return None;
    } else {
        return None;
    };
    parse_approval_request(id, real_method, inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::JsonRpcMessage;

    #[test]
    fn detects_approval_methods() {
        assert!(is_approval_method(EXEC_COMMAND_APPROVAL));
        assert!(is_approval_method(APPLY_PATCH_APPROVAL));
        assert!(is_approval_method(ITEM_COMMAND_EXECUTION_REQUEST_APPROVAL));
        assert!(is_approval_method(ITEM_FILE_CHANGE_REQUEST_APPROVAL));
        assert!(is_approval_method(ITEM_PERMISSIONS_REQUEST_APPROVAL));
        assert!(!is_approval_method("turn/started"));
        assert!(!is_approval_method("item/tool/call"));
    }

    #[test]
    fn parses_exec_command_approval_server_request() {
        let line = r#"{"id":42,"method":"execCommandApproval","params":{"conversationId":"thr-1","callId":"call-9","approvalId":null,"command":["ls","-la","/tmp"],"cwd":"/home/proj","reason":"inspect workspace","parsedCmd":[{"type":"list_files","cmd":"ls -la /tmp","path":"/tmp"}]}}"#;
        let msg = JsonRpcMessage::parse_line(line).unwrap();
        match msg {
            JsonRpcMessage::ServerRequest { id, method, params } => {
                assert_eq!(method, EXEC_COMMAND_APPROVAL);
                let pending = parse_approval_request(id, &method, params.as_ref()).unwrap();
                assert_eq!(pending.kind, ApprovalKind::ExecCommand);
                assert_eq!(pending.summary, "ls -la /tmp");
                assert!(pending.detail.contains("/home/proj"));
                assert_eq!(pending.thread_id.as_deref(), Some("thr-1"));
            }
            other => panic!("expected ServerRequest, got {other:?}"),
        }
    }

    #[test]
    fn parses_apply_patch_approval() {
        let line = r##"{"id":"req-patch-1","method":"applyPatchApproval","params":{"conversationId":"thr-2","callId":"c1","fileChanges":{"src/main.rs":{"type":"update","unified_diff":"@@ -1 +1 @@\n-old\n+new\n","move_path":null},"README.md":{"type":"add","content":"# Hi\n"}},"reason":"fix intro","grantRoot":null}}"##;
        let msg = JsonRpcMessage::parse_line(line).unwrap();
        let JsonRpcMessage::ServerRequest { id, method, params } = msg else {
            panic!("not server request");
        };
        let pending = parse_approval_request(id, &method, params.as_ref()).unwrap();
        assert_eq!(pending.kind, ApprovalKind::ApplyPatch);
        assert!(
            pending.summary.contains("files")
                || pending.summary.contains("README")
                || pending.summary.contains("main.rs")
        );
        assert!(pending.detail.contains("main.rs") || pending.detail.contains("README"));
    }

    #[test]
    fn parses_v2_command_execution_request_approval() {
        let line = r#"{"id":7,"method":"item/commandExecution/requestApproval","params":{"itemId":"item-cmd-1","threadId":"fixture-thread","turnId":"turn-1","startedAtMs":1000,"command":"cargo test -p mitsuro-desktop-backend","cwd":"/tmp/mitsuro-fixture","reason":"run unit tests","approvalId":null}}"#;
        let msg = JsonRpcMessage::parse_line(line).unwrap();
        let JsonRpcMessage::ServerRequest { id, method, params } = msg else {
            panic!("not server request");
        };
        let pending = parse_approval_request(id, &method, params.as_ref()).unwrap();
        assert_eq!(pending.kind, ApprovalKind::CommandExecution);
        assert_eq!(pending.summary, "cargo test -p mitsuro-desktop-backend");
        assert_eq!(pending.thread_id.as_deref(), Some("fixture-thread"));
        assert_eq!(pending.turn_id.as_deref(), Some("turn-1"));
    }

    #[test]
    fn review_decision_approve_deny_shapes() {
        let approve = build_approval_result(ApprovalKind::ExecCommand, ApprovalChoice::Approve);
        assert_eq!(approve["decision"], json!("approved"));

        let deny = build_approval_result(ApprovalKind::ApplyPatch, ApprovalChoice::Reject);
        assert_eq!(deny["decision"]["denied"]["rejection"], "User rejected");

        let abort = build_approval_result(ApprovalKind::ExecCommand, ApprovalChoice::Abort);
        assert_eq!(abort["decision"], json!("abort"));

        // Round-trip through typed structs
        let resp: ReviewDecisionResponse = serde_json::from_value(approve).unwrap();
        assert_eq!(
            resp.decision,
            ReviewDecision::Simple(ReviewDecisionSimple::Approved)
        );
        let deny_typed: ReviewDecisionResponse = serde_json::from_value(deny).unwrap();
        match deny_typed.decision {
            ReviewDecision::Denied { denied } => assert_eq!(denied.rejection, "User rejected"),
            other => panic!("expected Denied, got {other:?}"),
        }
    }

    #[test]
    fn v2_command_execution_decision_shapes() {
        let accept = build_approval_result(ApprovalKind::CommandExecution, ApprovalChoice::Approve);
        assert_eq!(accept["decision"], json!("accept"));

        let decline = build_approval_result(ApprovalKind::CommandExecution, ApprovalChoice::Reject);
        assert_eq!(decline["decision"], json!("decline"));

        let cancel = build_approval_result(ApprovalKind::CommandExecution, ApprovalChoice::Abort);
        assert_eq!(cancel["decision"], json!("cancel"));

        let file_accept = build_approval_result(ApprovalKind::FileChange, ApprovalChoice::Approve);
        assert_eq!(file_accept["decision"], json!("accept"));
        let file_decline = build_approval_result(ApprovalKind::FileChange, ApprovalChoice::Reject);
        assert_eq!(file_decline["decision"], json!("decline"));
    }

    #[test]
    fn permission_approval_echoes_only_the_requested_profile() {
        let params = json!({
            "threadId": "t1",
            "turnId": "u1",
            "itemId": "i1",
            "startedAtMs": 10,
            "cwd": "/repo",
            "permissions": {
                "fileSystem": { "read": ["/repo/data"] },
                "network": { "enabled": true }
            },
            "reason": "read inputs"
        });
        let pending = parse_approval_request(
            JsonRpcId::Number(12),
            ITEM_PERMISSIONS_REQUEST_APPROVAL,
            Some(&params),
        )
        .expect("permission approval");
        assert_eq!(pending.kind, ApprovalKind::Permissions);
        assert_eq!(pending.summary, "filesystem and network");
        let approved = build_pending_approval_result(&pending, ApprovalChoice::Approve);
        assert_eq!(approved["permissions"], params["permissions"]);
        assert_eq!(approved["scope"], "turn");
        let rejected = build_pending_approval_result(&pending, ApprovalChoice::Reject);
        assert_eq!(rejected["permissions"], json!({}));
    }

    #[test]
    fn rpc_response_includes_id_and_result() {
        let id = JsonRpcId::Number(99);
        let body = build_approval_rpc_response(
            &id,
            ApprovalKind::CommandExecution,
            ApprovalChoice::Approve,
        );
        assert_eq!(body["id"], 99);
        assert_eq!(body["result"]["decision"], "accept");
        assert_eq!(body["jsonrpc"], "2.0");
    }

    #[test]
    fn pending_from_pseudo_notification() {
        let params = json!({
            "id": 5,
            "params": {
                "itemId": "i1",
                "threadId": "t1",
                "turnId": "u1",
                "startedAtMs": 1,
                "command": "echo hi",
                "cwd": "/tmp"
            }
        });
        let pending = pending_from_server_request_notification(
            "serverRequest/item/commandExecution/requestApproval",
            Some(&params),
        )
        .unwrap();
        assert_eq!(pending.summary, "echo hi");
        assert_eq!(pending.request_id, JsonRpcId::Number(5));
    }
}
