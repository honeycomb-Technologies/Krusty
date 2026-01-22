//! ACP tool integration
//!
//! Bridges Krusty's tool system with ACP's tool call protocol.
//! Some tools are executed locally, others are delegated to the client.

use agent_client_protocol::{
    Client, Content, ContentBlock, CreateTerminalRequest, Diff, ReadTextFileRequest,
    ReleaseTerminalRequest, SessionId, TerminalId, TerminalOutputRequest, TextContent,
    ToolCallContent, ToolCallId, ToolCallLocation, ToolCallStatus, ToolCallUpdate,
    ToolCallUpdateFields, ToolKind, WriteTextFileRequest,
};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

use super::error::AcpError;

/// Tools that should be delegated to the client (editor)
#[allow(dead_code)]
const CLIENT_DELEGATED_TOOLS: &[&str] = &[
    // File operations - editor has the authoritative view
    // Note: We may still execute locally if client doesn't support
];

/// Check if a tool should be delegated to the client
#[allow(dead_code)]
pub fn should_delegate_to_client(tool_name: &str) -> bool {
    CLIENT_DELEGATED_TOOLS.contains(&tool_name)
}

/// Map tool name to ACP ToolKind for proper UI categorization
pub fn tool_name_to_kind(tool_name: &str) -> ToolKind {
    match tool_name {
        // Read operations
        "read" | "Read" | "cat" => ToolKind::Read,
        // Edit operations
        "edit" | "Edit" | "write" | "Write" | "patch" => ToolKind::Edit,
        // Search operations
        "grep" | "Grep" | "glob" | "Glob" | "find" | "search" | "ripgrep" => ToolKind::Search,
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

    // For grep/glob, extract the search path
    if matches!(tool_name, "grep" | "Grep" | "glob" | "Glob") {
        if let Some(path_str) = input.get("directory").and_then(|v| v.as_str()) {
            locations.push(ToolCallLocation::new(PathBuf::from(path_str)));
        }
    }

    locations
}

/// Create a human-readable title for a tool call
pub fn create_tool_title(tool_name: &str, input: &Value) -> String {
    match tool_name {
        "read" | "Read" => {
            if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
                let filename = Path::new(path)
                    .file_name()
                    .map(|s| s.to_string_lossy())
                    .unwrap_or_else(|| path.into());
                format!("Reading {}", filename)
            } else {
                "Reading file".to_string()
            }
        }
        "edit" | "Edit" => {
            if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
                let filename = Path::new(path)
                    .file_name()
                    .map(|s| s.to_string_lossy())
                    .unwrap_or_else(|| path.into());
                format!("Editing {}", filename)
            } else {
                "Editing file".to_string()
            }
        }
        "write" | "Write" => {
            if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
                let filename = Path::new(path)
                    .file_name()
                    .map(|s| s.to_string_lossy())
                    .unwrap_or_else(|| path.into());
                format!("Writing {}", filename)
            } else {
                "Writing file".to_string()
            }
        }
        "bash" | "Bash" => {
            if let Some(cmd) = input.get("command").and_then(|v| v.as_str()) {
                // Truncate long commands
                let display = if cmd.len() > 50 {
                    format!("{}...", &cmd[..47])
                } else {
                    cmd.to_string()
                };
                format!("Running: {}", display)
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
                let display = if url.len() > 40 {
                    format!("{}...", &url[..37])
                } else {
                    url.to_string()
                };
                format!("Fetching: {}", display)
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

/// Convert a file diff to ACP diff content
/// Note: ACP Diff only stores the new text, not old text (unified diff is computed by client)
#[allow(dead_code)]
pub fn diff_to_tool_content(path: &Path, _old_text: &str, new_text: &str) -> ToolCallContent {
    ToolCallContent::Diff(Diff::new(
        path.to_string_lossy().to_string(),
        new_text.to_string(),
    ))
}

/// Read a file via the ACP client (if supported)
#[allow(dead_code)]
pub async fn read_file_via_client<C: Client>(
    client: &C,
    session_id: &SessionId,
    path: &Path,
    _start_line: Option<u32>,
    _end_line: Option<u32>,
) -> Result<String, AcpError> {
    debug!("Reading file via client: {:?}", path);

    let request = ReadTextFileRequest::new(session_id.clone(), path.to_string_lossy().to_string());

    match client.read_text_file(request).await {
        Ok(response) => Ok(response.content),
        Err(e) => {
            warn!("Client file read failed, will fallback to local: {}", e);
            Err(AcpError::ToolError(e.to_string()))
        }
    }
}

/// Write a file via the ACP client (if supported)
#[allow(dead_code)]
pub async fn write_file_via_client<C: Client>(
    client: &C,
    session_id: &SessionId,
    path: &Path,
    content: &str,
) -> Result<(), AcpError> {
    debug!("Writing file via client: {:?}", path);

    let request = WriteTextFileRequest::new(
        session_id.clone(),
        path.to_string_lossy().to_string(),
        content.to_string(),
    );

    client
        .write_text_file(request)
        .await
        .map_err(|e| AcpError::ToolError(e.to_string()))?;

    Ok(())
}

/// Create a terminal via the ACP client
#[allow(dead_code)]
pub async fn create_terminal_via_client<C: Client>(
    client: &C,
    session_id: &SessionId,
    command: &str,
    _cwd: Option<&Path>,
) -> Result<String, AcpError> {
    debug!("Creating terminal via client: {}", command);

    let request = CreateTerminalRequest::new(session_id.clone(), command.to_string());

    let response = client
        .create_terminal(request)
        .await
        .map_err(|e| AcpError::ToolError(e.to_string()))?;

    Ok(response.terminal_id.to_string())
}

/// Get terminal output via the ACP client
#[allow(dead_code)]
pub async fn get_terminal_output_via_client<C: Client>(
    client: &C,
    session_id: &SessionId,
    terminal_id: &str,
) -> Result<(String, bool), AcpError> {
    debug!("Getting terminal output: {}", terminal_id);

    let request = TerminalOutputRequest::new(
        session_id.clone(),
        TerminalId::from(terminal_id.to_string()),
    );

    let response = client
        .terminal_output(request)
        .await
        .map_err(|e| AcpError::ToolError(e.to_string()))?;

    let is_complete = response.exit_status.is_some();
    Ok((response.output, is_complete))
}

/// Release a terminal via the ACP client
#[allow(dead_code)]
pub async fn release_terminal_via_client<C: Client>(
    client: &C,
    session_id: &SessionId,
    terminal_id: &str,
) -> Result<(), AcpError> {
    debug!("Releasing terminal: {}", terminal_id);

    let request = ReleaseTerminalRequest::new(
        session_id.clone(),
        TerminalId::from(terminal_id.to_string()),
    );

    client
        .release_terminal(request)
        .await
        .map_err(|e| AcpError::ToolError(e.to_string()))?;

    Ok(())
}
