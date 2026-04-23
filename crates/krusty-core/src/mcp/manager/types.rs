use serde_json::Value;

/// Server status
#[derive(Debug, Clone, PartialEq)]
pub enum McpServerStatus {
    Disconnected,
    Connected,
    Error(String),
}

impl std::fmt::Display for McpServerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpServerStatus::Disconnected => write!(f, "disconnected"),
            McpServerStatus::Connected => write!(f, "connected"),
            McpServerStatus::Error(e) => write!(f, "error: {}", e),
        }
    }
}

/// Tool definition exposed to callers (bridge from rmcp::model::Tool)
#[derive(Debug, Clone)]
pub struct McpToolDef {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Value,
}

impl From<rmcp::model::Tool> for McpToolDef {
    fn from(tool: rmcp::model::Tool) -> Self {
        Self {
            name: tool.name.to_string(),
            description: tool.description.as_deref().map(|s| s.to_string()),
            input_schema: serde_json::to_value(&*tool.input_schema)
                .unwrap_or(Value::Object(serde_json::Map::new())),
        }
    }
}

/// Tool result exposed to callers (bridge from rmcp::model::CallToolResult)
#[derive(Debug, Clone)]
pub struct McpToolResult {
    pub content: Vec<McpContent>,
    pub is_error: bool,
}

/// Content types returned by MCP tools
#[derive(Debug, Clone)]
pub enum McpContent {
    Text { text: String },
    Image { data: String, mime_type: String },
    Resource { uri: String, text: Option<String> },
}

impl std::fmt::Display for McpContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpContent::Text { text } => write!(f, "{}", text),
            McpContent::Image { mime_type, .. } => write!(f, "[Image: {}]", mime_type),
            McpContent::Resource { uri, text } => {
                if let Some(t) = text {
                    write!(f, "{}\n{}", uri, t)
                } else {
                    write!(f, "{}", uri)
                }
            }
        }
    }
}

pub fn format_mcp_result(result: &McpToolResult) -> String {
    let mut formatted = String::new();
    for (idx, content) in result.content.iter().enumerate() {
        if idx > 0 {
            formatted.push('\n');
        }
        formatted.push_str(&content.to_string());
    }
    formatted
}

impl From<rmcp::model::CallToolResult> for McpToolResult {
    fn from(result: rmcp::model::CallToolResult) -> Self {
        let content = result
            .content
            .into_iter()
            .filter_map(|c| {
                use rmcp::model::RawContent;
                match c.raw {
                    RawContent::Text(text_content) => Some(McpContent::Text {
                        text: text_content.text,
                    }),
                    RawContent::Image(image_content) => Some(McpContent::Image {
                        data: image_content.data,
                        mime_type: image_content.mime_type,
                    }),
                    RawContent::Resource(embedded) => {
                        use rmcp::model::ResourceContents;
                        match embedded.resource {
                            ResourceContents::TextResourceContents { uri, text, .. } => {
                                Some(McpContent::Resource {
                                    uri,
                                    text: Some(text),
                                })
                            }
                            ResourceContents::BlobResourceContents { uri, .. } => {
                                Some(McpContent::Resource { uri, text: None })
                            }
                        }
                    }
                    _ => None,
                }
            })
            .collect();

        Self {
            content,
            is_error: result.is_error.unwrap_or(false),
        }
    }
}

/// Server info for UI
#[derive(Debug, Clone)]
pub struct McpServerInfo {
    pub name: String,
    pub server_type: String,
    pub status: McpServerStatus,
    pub tool_count: usize,
    pub tools: Vec<McpToolDef>,
    pub error: Option<String>,
}
