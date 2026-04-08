//! ACP tool integration
//!
//! Bridges Krusty's tool system with ACP's tool call protocol.

use agent_client_protocol::{
    Content, ContentBlock, TextContent, ToolCallContent, ToolCallId, ToolCallLocation,
    ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, ToolKind,
};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Map tool name to ACP ToolKind for proper UI categorization
pub fn tool_name_to_kind(tool_name: &str) -> ToolKind {
    match tool_name {
        // Read operations
        "read" | "Read" | "cat" => ToolKind::Read,
        // Edit operations
        "edit" | "Edit" | "write" | "Write" | "patch" | "apply_patch" | "multiedit"
        | "multi_edit" => ToolKind::Edit,
        // Search operations
        "grep" | "Grep" | "glob" | "Glob" | "find" | "search" | "ripgrep" | "list" | "ls"
        | "list_dir" => ToolKind::Search,
        // Execute operations
        "bash" | "Bash" | "shell" | "exec" | "run" | "terminal" => ToolKind::Execute,
        // Fetch operations
        "web_fetch" | "WebFetch" | "fetch" | "curl" | "http" | "web_search" | "WebSearch" => {
            ToolKind::Fetch
        }
        // Think operations
        "think" | "reason" | "plan" => ToolKind::Think,
        // Delete operations
        "delete" | "remove" | "rm" => ToolKind::Delete,
        // Move operations
        "move" | "mv" | "rename" => ToolKind::Move,
        // Default
        _ => ToolKind::Other,
    }
}

/// Extract file locations from tool input for "follow-along" feature
pub fn extract_locations(tool_name: &str, input: &Value) -> Vec<ToolCallLocation> {
    let mut locations = Vec::new();

    // Extract path from common field names
    let path_fields = ["path", "file_path", "file", "filename"];
    for field in path_fields {
        if let Some(path_str) = input.get(field).and_then(|v| v.as_str()) {
            let mut loc = ToolCallLocation::new(PathBuf::from(path_str));
            // Try to extract line number if present
            if let Some(line) = input.get("line").and_then(|v| v.as_u64()) {
                loc = loc.line(line as u32);
            } else if let Some(line) = input.get("start_line").and_then(|v| v.as_u64()) {
                loc = loc.line(line as u32);
            }
            locations.push(loc);
            break;
        }
    }

    // For grep/glob/list, extract the search path
    if matches!(tool_name, "grep" | "Grep" | "glob" | "Glob" | "list") {
        if let Some(path_str) = input.get("directory").and_then(|v| v.as_str()) {
            locations.push(ToolCallLocation::new(PathBuf::from(path_str)));
        }
    }

    locations
}

fn file_display_name(path: &str) -> std::borrow::Cow<'_, str> {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| path.into())
}

fn truncate_for_title(value: &str, max_chars: usize) -> std::borrow::Cow<'_, str> {
    let mut chars = value.chars();
    let visible: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_none() {
        value.into()
    } else {
        format!("{}...", visible.trim_end_matches(char::is_whitespace)).into()
    }
}

fn titled_path_action(input: &Value, field: &str, verb: &str, fallback: &str) -> String {
    input
        .get(field)
        .and_then(Value::as_str)
        .map(file_display_name)
        .map(|name| format!("{verb} {name}"))
        .unwrap_or_else(|| fallback.to_string())
}

/// Create a human-readable title for a tool call
pub fn create_tool_title(tool_name: &str, input: &Value) -> String {
    match tool_name {
        "read" | "Read" => titled_path_action(input, "file_path", "Reading", "Reading file"),
        "edit" | "Edit" => titled_path_action(input, "file_path", "Editing", "Editing file"),
        "write" | "Write" => titled_path_action(input, "file_path", "Writing", "Writing file"),
        "bash" | "Bash" => {
            if let Some(cmd) = input.get("command").and_then(|v| v.as_str()) {
                format!("Running: {}", truncate_for_title(cmd, 47))
            } else {
                "Running command".to_string()
            }
        }
        "grep" | "Grep" => {
            if let Some(pattern) = input.get("pattern").and_then(|v| v.as_str()) {
                format!("Searching for: {}", pattern)
            } else {
                "Searching".to_string()
            }
        }
        "glob" | "Glob" => {
            if let Some(pattern) = input.get("pattern").and_then(|v| v.as_str()) {
                format!("Finding files: {}", pattern)
            } else {
                "Finding files".to_string()
            }
        }
        "web_fetch" | "WebFetch" => {
            if let Some(url) = input.get("url").and_then(|v| v.as_str()) {
                format!("Fetching: {}", truncate_for_title(url, 37))
            } else {
                "Fetching URL".to_string()
            }
        }
        "web_search" | "WebSearch" => {
            if let Some(query) = input.get("query").and_then(|v| v.as_str()) {
                format!("Searching web: {}", query)
            } else {
                "Searching web".to_string()
            }
        }
        "list" => titled_path_action(input, "path", "Listing", "Listing directory"),
        "apply_patch" => {
            if let Some(patch) = input.get("patch").and_then(|v| v.as_str()) {
                let file_count = patch.matches("*** Update File:").count()
                    + patch.matches("*** Add File:").count()
                    + patch.matches("*** Delete File:").count();
                if file_count > 0 {
                    format!("Patching {} files", file_count)
                } else {
                    "Applying patch".to_string()
                }
            } else {
                "Applying patch".to_string()
            }
        }
        "multiedit" | "multi_edit" => {
            if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
                let edit_count = input
                    .get("edits")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                format!("Editing {} ({} edits)", file_display_name(path), edit_count)
            } else {
                "Multi-editing file".to_string()
            }
        }
        _ => format!("Running {}", tool_name),
    }
}

/// Create an ACP tool call update for a starting tool
pub fn create_tool_call_start(id: &str, tool_name: &str, input: Value) -> ToolCallUpdate {
    let kind = tool_name_to_kind(tool_name);
    let locations = extract_locations(tool_name, &input);
    let title = create_tool_title(tool_name, &input);

    let mut fields = ToolCallUpdateFields::new();
    fields.status = Some(ToolCallStatus::InProgress);
    fields.kind = Some(kind);
    fields.title = Some(title);
    fields.raw_input = Some(input);
    if !locations.is_empty() {
        fields.locations = Some(locations);
    }

    ToolCallUpdate::new(ToolCallId::from(id.to_string()), fields)
}

/// Create an ACP tool call update for a completed tool
pub fn create_tool_call_complete(id: &str, content: Vec<ToolCallContent>) -> ToolCallUpdate {
    let mut fields = ToolCallUpdateFields::new();
    fields.status = Some(ToolCallStatus::Completed);
    fields.content = Some(content);

    ToolCallUpdate::new(ToolCallId::from(id.to_string()), fields)
}

/// Create an ACP tool call update for a failed tool
pub fn create_tool_call_failed(id: &str, error_message: &str) -> ToolCallUpdate {
    let error_content = ToolCallContent::Content(Content::new(ContentBlock::Text(
        TextContent::new(format!("Error: {}", error_message)),
    )));

    let mut fields = ToolCallUpdateFields::new();
    fields.status = Some(ToolCallStatus::Failed);
    fields.content = Some(vec![error_content]);

    ToolCallUpdate::new(ToolCallId::from(id.to_string()), fields)
}

/// Convert tool result text to ACP content
pub fn text_to_tool_content(text: &str) -> ToolCallContent {
    ToolCallContent::Content(Content::new(ContentBlock::Text(TextContent::new(text))))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::create_tool_title;

    #[test]
    fn create_tool_title_uses_file_name_for_path_actions() {
        assert_eq!(
            create_tool_title("read", &json!({"file_path": "/tmp/example.txt"})),
            "Reading example.txt"
        );
        assert_eq!(
            create_tool_title("list", &json!({"path": "/tmp/project"})),
            "Listing project"
        );
    }

    #[test]
    fn create_tool_title_truncates_unicode_safely() {
        let title = create_tool_title(
            "bash",
            &json!({"command": "echo 你好世界你好世界你好世界你好世界你好世界你好世界你好世界你好世界你好世界你好世界你好世界你好世界"}),
        );

        assert!(title.starts_with("Running: echo 你好世界"));
        assert!(title.ends_with("..."));
    }
}
