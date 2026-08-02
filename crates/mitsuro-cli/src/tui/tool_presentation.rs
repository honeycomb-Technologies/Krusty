//! Tool presentation policy for the TUI.
//!
//! This module is the single place that decides whether a tool should create a
//! chat widget, update existing UI only, or stay invisible. It also normalizes
//! raw tool outputs and persisted history summaries back into a renderable shape
//! so session replay does not leak protocol/history JSON.

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolPresentation {
    Bash,
    Read,
    Edit,
    Write,
    Search,
    WebSearch,
    ExploreAgent,
    BuildAgent,
    GenericStatus,
    UiOnly,
}

impl ToolPresentation {
    pub fn is_ui_only(self) -> bool {
        matches!(self, Self::UiOnly)
    }
}

/// Return the presentation policy for a tool call.
///
/// The policy intentionally does not mirror the tool registry one-to-one:
/// internal bookkeeping tools update existing UI state only, while user-relevant
/// work gets a styled block/chip.
pub fn presentation_for_tool(name: &str, input: &Value) -> ToolPresentation {
    match name {
        "bash" => ToolPresentation::Bash,
        "read" => ToolPresentation::Read,
        "edit" => ToolPresentation::Edit,
        "write" => ToolPresentation::Write,
        "grep" | "glob" | "list" => ToolPresentation::Search,
        "web_search" => ToolPresentation::WebSearch,
        "explore" | "Task" => ToolPresentation::ExploreAgent,
        "build" => ToolPresentation::BuildAgent,
        "agent" if agent_has_capability(input, "write") => ToolPresentation::BuildAgent,
        "agent"
            if input.get("name").is_some()
                || input.get("instructions").is_some()
                || input.get("capabilities").is_some() =>
        {
            ToolPresentation::ExploreAgent
        }
        "agent" => match input
            .get("agent_type")
            .or_else(|| input.get("profile"))
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "explore" => ToolPresentation::ExploreAgent,
            "build" => ToolPresentation::BuildAgent,
            _ => ToolPresentation::GenericStatus,
        },
        // Plan/process/workspace actions have their own UI surfaces. A chat
        // widget would be duplicate noise.
        "task_start"
        | "task_complete"
        | "add_subtask"
        | "set_dependency"
        | "set_work_mode"
        | "enter_plan_mode"
        | "set_workspace_context"
        | "todowrite"
        | "processes"
        | "AskUserQuestion"
        | "PlanConfirm" => ToolPresentation::UiOnly,
        // Everything else gets a compact semantic status chip instead of raw
        // text/JSON. This includes MCP tools and future extensions by default.
        _ => ToolPresentation::GenericStatus,
    }
}

/// A parsed view of a tool output.
#[derive(Debug, Clone)]
pub struct ToolOutputView {
    /// Best-effort payload to feed existing widgets. This strips Mitsuro history
    /// wrappers (`{ tool, retention, summary, result }`) when present.
    pub render_output: String,
    /// Human summary for compact/generic rows.
    pub summary: Option<String>,
    /// Whether the persisted history wrapper marked the tool as an error.
    pub is_error: Option<bool>,
}

impl ToolOutputView {
    pub fn from_output(output: &str) -> Self {
        let Ok(parsed) = serde_json::from_str::<Value>(output) else {
            return Self {
                render_output: output.to_string(),
                summary: first_non_empty_line(output),
                is_error: None,
            };
        };

        // Persisted history shape from build_history_tool_result(). This is
        // model-facing metadata, not a UI contract.
        if parsed.get("retention").is_some() && parsed.get("result").is_some() {
            let result = parsed.get("result").unwrap_or(&Value::Null);
            let render_output = match result {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            return Self {
                render_output,
                summary: parsed
                    .get("summary")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                is_error: parsed.get("is_error").and_then(Value::as_bool),
            };
        }

        let summary = extract_summary(&parsed).or_else(|| first_non_empty_line(output));
        let is_error = parsed
            .get("ok")
            .and_then(Value::as_bool)
            .map(|ok| !ok)
            .or_else(|| parsed.get("error").map(|_| true));

        Self {
            render_output: output.to_string(),
            summary,
            is_error,
        }
    }
}

pub fn renderable_tool_output(output: &str) -> String {
    ToolOutputView::from_output(output).render_output
}

pub fn tool_summary(output: &str) -> Option<String> {
    ToolOutputView::from_output(output).summary
}

pub fn payload_for_render(output: &str) -> Option<Value> {
    let view = ToolOutputView::from_output(output);
    serde_json::from_str::<Value>(&view.render_output).ok()
}

pub fn first_string(input: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| input.get(*key).and_then(Value::as_str))
        .map(ToString::to_string)
}

pub fn tool_pattern(name: &str, input: &Value) -> String {
    match name {
        "grep" | "glob" => first_string(input, &["pattern"]).unwrap_or_default(),
        "list" => first_string(input, &["path"]).unwrap_or_else(|| ".".to_string()),
        "web_search" => first_string(input, &["query"]).unwrap_or_default(),
        "web_fetch" => first_string(input, &["url"]).unwrap_or_default(),
        "multiedit" => first_string(input, &["file_path"]).unwrap_or_default(),
        "apply_patch" => "patch".to_string(),
        "memory" | "report" | "autonomous_task" | "processes" => {
            first_string(input, &["action"]).unwrap_or_default()
        }
        "skill" => first_string(input, &["skill"]).unwrap_or_default(),
        "search_compaction_segments" => first_string(input, &["query"]).unwrap_or_default(),
        "agent" => first_string(input, &["name"])
            .or_else(|| agent_capability_label(input))
            .or_else(|| first_string(input, &["agent_type", "profile"]))
            .unwrap_or_default(),
        _ => String::new(),
    }
}

pub fn display_tool_name(name: &str, input: &Value) -> String {
    match name {
        "agent" if first_string(input, &["name"]).is_some() => {
            first_string(input, &["name"]).unwrap_or_else(|| "agent".to_string())
        }
        "agent" => match input
            .get("agent_type")
            .or_else(|| input.get("profile"))
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "explore" => "explore".to_string(),
            "build" => "build".to_string(),
            "plan" => "agent plan".to_string(),
            "verify" => "agent verify".to_string(),
            _ => "agent".to_string(),
        },
        "search_compaction_segments" => "compaction search".to_string(),
        "send_user_message" => "user message".to_string(),
        "web_fetch" => "web fetch".to_string(),
        "multiedit" => "multi edit".to_string(),
        "apply_patch" => "patch".to_string(),
        other => other.replace('_', " "),
    }
}

fn agent_has_capability(input: &Value, expected: &str) -> bool {
    input
        .get("capabilities")
        .and_then(Value::as_array)
        .is_some_and(|capabilities| {
            capabilities
                .iter()
                .filter_map(Value::as_str)
                .any(|capability| capability == expected)
        })
}

fn agent_capability_label(input: &Value) -> Option<String> {
    let capabilities = input.get("capabilities")?.as_array()?;
    let values = capabilities
        .iter()
        .filter_map(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>();
    (!values.is_empty()).then(|| values.join(" + "))
}

fn extract_summary(parsed: &Value) -> Option<String> {
    if let Some(message) = parsed
        .get("error")
        .and_then(|value| value.get("message"))
        .and_then(Value::as_str)
    {
        return Some(message.to_string());
    }

    let payload = parsed.get("data").unwrap_or(parsed);
    for key in [
        "message",
        "summary",
        "investigation_summary",
        "title",
        "subject",
    ] {
        if let Some(value) = payload.get(key).and_then(Value::as_str) {
            return Some(value.to_string());
        }
    }

    if let Some(count) = payload
        .get("result_count")
        .or_else(|| payload.get("count"))
        .or_else(|| payload.get("total_matches"))
        .and_then(Value::as_u64)
    {
        return Some(format!(
            "{} result{}",
            count,
            if count == 1 { "" } else { "s" }
        ));
    }

    None
}

fn first_non_empty_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(220).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strips_history_wrapper_for_rendering() {
        let wrapped = json!({
            "tool": "grep",
            "retention": "summarize_after_turn",
            "summary": "grep found 2 matches",
            "is_error": false,
            "result": { "total_matches": 2, "matches": [] }
        })
        .to_string();

        let view = ToolOutputView::from_output(&wrapped);

        assert_eq!(view.summary.as_deref(), Some("grep found 2 matches"));
        assert_eq!(view.is_error, Some(false));
        assert!(view.render_output.contains("total_matches"));
        assert!(!view.render_output.contains("retention"));
    }

    #[test]
    fn plan_and_process_tools_are_ui_only() {
        assert!(presentation_for_tool("task_complete", &json!({})).is_ui_only());
        assert!(presentation_for_tool("processes", &json!({"action":"kill"})).is_ui_only());
    }

    #[test]
    fn agent_type_selects_existing_widget_family() {
        assert_eq!(
            presentation_for_tool("agent", &json!({"agent_type":"explore"})),
            ToolPresentation::ExploreAgent
        );
        assert_eq!(
            presentation_for_tool("agent", &json!({"agent_type":"build"})),
            ToolPresentation::BuildAgent
        );
        assert_eq!(
            presentation_for_tool("agent", &json!({"agent_type":"verify"})),
            ToolPresentation::GenericStatus
        );
    }

    #[test]
    fn current_agent_contract_uses_name_and_exact_capabilities() {
        let input = json!({
            "name": "focused validator",
            "instructions": "run focused checks",
            "capabilities": ["execute"]
        });
        assert_eq!(
            presentation_for_tool("agent", &input),
            ToolPresentation::ExploreAgent
        );
        assert_eq!(display_tool_name("agent", &input), "focused validator");
        assert_eq!(tool_pattern("agent", &input), "focused validator");

        let unnamed = json!({"capabilities": ["read", "execute"]});
        assert_eq!(tool_pattern("agent", &unnamed), "read + execute");
        let writer = json!({"name": "repair", "capabilities": ["write"]});
        assert_eq!(
            presentation_for_tool("agent", &writer),
            ToolPresentation::BuildAgent
        );
    }

    #[test]
    fn multiedit_has_explicit_status_label_and_file_pattern() {
        let input = json!({"file_path":"src/lib.rs"});

        assert_eq!(
            presentation_for_tool("multiedit", &input),
            ToolPresentation::GenericStatus
        );
        assert_eq!(display_tool_name("multiedit", &input), "multi edit");
        assert_eq!(tool_pattern("multiedit", &input), "src/lib.rs");
    }
}
