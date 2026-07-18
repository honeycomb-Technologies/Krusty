use krusty_core::storage::{SessionType, WorkMode, WorkspaceMode};
use krusty_core::tools::registry::PermissionMode;
use serde::{de, Deserialize, Deserializer};

// ============================================================================
// Chat Types
// ============================================================================

/// Content block from web/mobile clients (text or image)
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ContentBlock {
    Text { text: String },
    Image { source: ImageSource },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ImageSource {
    Base64 { media_type: String, data: String },
    Url { url: String },
}

/// Extended thinking level (accepts legacy bool and newer string levels from clients).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThinkingLevel {
    #[default]
    Off,
    Low,
    Medium,
    High,
    XHigh,
}

impl ThinkingLevel {
    pub fn is_enabled(self) -> bool {
        !matches!(self, Self::Off)
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ThinkingLevelInput {
    Bool(bool),
    String(String),
}

fn deserialize_thinking_level<'de, D>(deserializer: D) -> Result<ThinkingLevel, D::Error>
where
    D: Deserializer<'de>,
{
    let input = Option::<ThinkingLevelInput>::deserialize(deserializer)?;
    match input {
        None => Ok(ThinkingLevel::Off),
        Some(ThinkingLevelInput::Bool(enabled)) => Ok(if enabled {
            ThinkingLevel::High
        } else {
            ThinkingLevel::Off
        }),
        Some(ThinkingLevelInput::String(raw)) => {
            let value = raw.trim().to_ascii_lowercase();
            match value.as_str() {
                "" | "off" | "false" | "disabled" | "none" => Ok(ThinkingLevel::Off),
                "on" | "true" | "enabled" => Ok(ThinkingLevel::High),
                "low" => Ok(ThinkingLevel::Low),
                "medium" => Ok(ThinkingLevel::Medium),
                "high" => Ok(ThinkingLevel::High),
                "xhigh" | "x-high" | "extra-high" => Ok(ThinkingLevel::XHigh),
                _ => Err(de::Error::custom(format!(
                    "invalid thinking_enabled value '{}'; expected bool or one of off/low/medium/high/xhigh",
                    raw
                ))),
            }
        }
    }
}

#[derive(Deserialize)]
pub struct ChatRequest {
    /// Session ID (creates new session if not provided)
    pub session_id: Option<String>,
    /// User message content (text fallback)
    pub message: String,
    /// Multi-modal content blocks (text + images)
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    /// Explicit project directory for newly created sessions.
    pub project_dir: Option<String>,
    /// Explicit workspace directory for newly created sessions. `None` means no project selected.
    /// Legacy alias for `project_dir`.
    pub working_dir: Option<String>,
    /// Explicit semantic workspace mode for newly created sessions.
    pub workspace_mode: Option<WorkspaceMode>,
    /// Optional target branch intent metadata. This does not checkout or mutate branches.
    #[serde(default, alias = "targetBranch")]
    pub target_branch: Option<String>,
    /// High-level session surface for newly created sessions.
    pub session_type: Option<SessionType>,
    /// Model override
    pub model: Option<String>,
    /// Enable extended thinking
    #[serde(default, deserialize_with = "deserialize_thinking_level")]
    pub thinking_enabled: ThinkingLevel,
    /// Optional mode override for the session before starting this turn
    pub mode: Option<WorkMode>,
    /// Permission mode for tool execution. If omitted for an existing session,
    /// the server uses the session's persisted mode.
    #[serde(default)]
    pub permission_mode: Option<PermissionMode>,
    /// Request provider fast/priority service tier without changing the selected model
    #[serde(default)]
    pub fast_mode: bool,
    /// Enable research tools (agent, reports) in Chat sessions
    #[serde(default)]
    pub research_enabled: Option<bool>,
}

#[derive(Deserialize)]
pub struct SteerRequest {
    pub session_id: String,
    pub message: String,
    #[serde(default)]
    pub content: Vec<ContentBlock>,
}

#[cfg(test)]
mod tests {
    use super::{ChatRequest, ThinkingLevel};
    use crate::types::AgenticEvent;
    use krusty_core::agent::LoopEvent;
    use krusty_core::storage::{RuntimeTraceEvent, WorkspaceMode};
    use serde_json::json;

    #[test]
    fn chat_request_accepts_legacy_bool_thinking() {
        let req: ChatRequest = serde_json::from_value(json!({
            "message": "hello",
            "thinking_enabled": true
        }))
        .expect("request should deserialize");
        assert_eq!(req.thinking_enabled, ThinkingLevel::High);
    }

    #[test]
    fn chat_request_accepts_string_thinking_level() {
        let req: ChatRequest = serde_json::from_value(json!({
            "message": "hello",
            "thinking_enabled": "medium"
        }))
        .expect("request should deserialize");
        assert_eq!(req.thinking_enabled, ThinkingLevel::Medium);
    }

    #[test]
    fn chat_request_defaults_thinking_to_off() {
        let req: ChatRequest = serde_json::from_value(json!({
            "message": "hello"
        }))
        .expect("request should deserialize");
        assert_eq!(req.thinking_enabled, ThinkingLevel::Off);
    }

    #[test]
    fn chat_request_accepts_fast_mode_flag() {
        let req: ChatRequest = serde_json::from_value(json!({
            "message": "hello",
            "fast_mode": true
        }))
        .expect("request should deserialize");

        assert!(req.fast_mode);
        assert_eq!(req.permission_mode, None);
    }

    #[test]
    fn chat_request_accepts_snake_case_target_branch() {
        let req: ChatRequest = serde_json::from_value(json!({
            "message": "hello",
            "target_branch": "feature/mobile-continuation"
        }))
        .expect("request should deserialize");

        assert_eq!(
            req.target_branch.as_deref(),
            Some("feature/mobile-continuation")
        );
    }

    #[test]
    fn chat_request_accepts_camel_case_target_branch_alias() {
        let req: ChatRequest = serde_json::from_value(json!({
            "message": "hello",
            "targetBranch": "feature/mobile-continuation"
        }))
        .expect("request should deserialize");

        assert_eq!(
            req.target_branch.as_deref(),
            Some("feature/mobile-continuation")
        );
    }

    #[test]
    fn tool_result_request_accepts_fast_mode_flag() {
        let req: super::ToolResultRequest = serde_json::from_value(json!({
            "session_id": "session-1",
            "tool_call_id": "tool-1",
            "result": "ok",
            "fast_mode": true
        }))
        .expect("request should deserialize");

        assert!(req.fast_mode);
        assert_eq!(req.permission_mode, None);
    }

    #[test]
    fn tool_result_request_accepts_permission_mode() {
        let req: super::ToolResultRequest = serde_json::from_value(json!({
            "session_id": "session-1",
            "tool_call_id": "tool-1",
            "result": "ok",
            "permission_mode": "autonomous"
        }))
        .expect("request should deserialize");

        assert_eq!(
            req.permission_mode,
            Some(krusty_core::tools::registry::PermissionMode::Autonomous)
        );
    }

    #[test]
    fn chat_request_accepts_explicit_no_project_working_dir() {
        let req: ChatRequest = serde_json::from_value(json!({
            "message": "hello",
            "working_dir": null
        }))
        .expect("request should deserialize");
        assert_eq!(req.working_dir, None);
        assert_eq!(req.project_dir, None);
        assert_eq!(req.workspace_mode, None);
    }

    #[test]
    fn chat_request_accepts_explicit_workspace_contract() {
        let req: ChatRequest = serde_json::from_value(json!({
            "message": "hello",
            "project_dir": "/tmp/demo",
            "workspace_mode": "created"
        }))
        .expect("request should deserialize");
        assert_eq!(req.project_dir.as_deref(), Some("/tmp/demo"));
        assert_eq!(req.workspace_mode, Some(WorkspaceMode::Created));
    }

    #[test]
    fn chat_request_rejects_invalid_thinking_value() {
        let result = serde_json::from_value::<ChatRequest>(json!({
            "message": "hello",
            "thinking_enabled": "turbo"
        }));
        match result {
            Ok(_) => panic!("request should fail"),
            Err(err) => assert!(err.to_string().contains("invalid thinking_enabled value")),
        }
    }

    #[test]
    fn agentic_event_preserves_thinking_complete_shape() {
        let mapped = AgenticEvent::from(LoopEvent::ThinkingComplete {
            thinking: "analysis".to_string(),
            signature: "sig-1".to_string(),
        });

        match mapped {
            AgenticEvent::ThinkingComplete {
                thinking,
                signature,
            } => {
                assert_eq!(thinking, "analysis");
                assert_eq!(signature, "sig-1");
            }
            other => panic!("unexpected mapping: {other:?}"),
        }
    }

    #[test]
    fn agentic_event_preserves_web_search_results_shape() {
        let mapped = AgenticEvent::from(LoopEvent::WebSearchResults {
            tool_use_id: "tool-1".to_string(),
            results: Vec::new(),
        });

        match mapped {
            AgenticEvent::WebSearchResults {
                tool_use_id,
                results,
            } => {
                assert_eq!(tool_use_id, "tool-1");
                assert!(results.is_empty());
            }
            other => panic!("unexpected mapping: {other:?}"),
        }
    }

    #[test]
    fn agentic_event_preserves_server_tool_error_shape() {
        let mapped = AgenticEvent::from(LoopEvent::ServerToolError {
            tool_use_id: "tool-7".to_string(),
            error_code: "timeout".to_string(),
        });

        match mapped {
            AgenticEvent::ServerToolError {
                tool_use_id,
                error_code,
            } => {
                assert_eq!(tool_use_id, "tool-7");
                assert_eq!(error_code, "timeout");
            }
            other => panic!("unexpected mapping: {other:?}"),
        }
    }
    #[test]
    fn agentic_event_replays_user_message_trace() {
        let event = RuntimeTraceEvent {
            run_id: "run-1".to_string(),
            sequence: 7,
            turn: 1,
            event_type: "user_message".to_string(),
            call_kind: None,
            operation: None,
            payload: json!({
                "title": "Milestone",
                "message": "Indexing finished",
                "level": "success"
            }),
            failure_category: None,
            stop_reason: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };

        match AgenticEvent::from_runtime_trace(event) {
            Some(AgenticEvent::UserMessage {
                title,
                message,
                level,
            }) => {
                assert_eq!(title.as_deref(), Some("Milestone"));
                assert_eq!(message, "Indexing finished");
                assert_eq!(level, "success");
            }
            other => panic!("unexpected replay mapping: {other:?}"),
        }
    }

    #[test]
    fn agentic_event_skips_unreplayable_trace_shapes() {
        let event = RuntimeTraceEvent {
            run_id: "run-1".to_string(),
            sequence: 8,
            turn: 1,
            event_type: "plan_update".to_string(),
            call_kind: None,
            operation: None,
            payload: json!({ "task_count": 3 }),
            failure_category: None,
            stop_reason: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };

        assert!(AgenticEvent::from_runtime_trace(event).is_none());
    }
}

#[derive(Deserialize)]
pub struct ToolResultRequest {
    /// Session ID
    pub session_id: String,
    /// Exact autonomous run that requested the response. Older clients may
    /// omit this; the server then resolves one unambiguous pending run.
    #[serde(default)]
    pub run_id: Option<String>,
    /// Tool use ID to respond to
    pub tool_call_id: String,
    /// Tool result content (JSON string)
    pub result: String,
    /// Request provider fast/priority service tier while resuming after a tool result
    #[serde(default)]
    pub fast_mode: bool,
    /// Permission mode to preserve while resuming after an interactive tool result.
    /// If omitted, the server uses recovery/session metadata.
    #[serde(default)]
    pub permission_mode: Option<PermissionMode>,
}

#[derive(Deserialize)]
pub struct ToolApprovalRequest {
    pub session_id: String,
    /// Exact autonomous run that emitted the approval request.
    #[serde(default)]
    pub run_id: Option<String>,
    pub tool_call_id: String,
    pub approved: bool,
}
