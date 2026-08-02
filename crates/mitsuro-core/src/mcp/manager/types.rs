use serde::Serialize;
use serde_json::Value;

use crate::mcp::config::{McpConfigSource, McpToolApproval};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpServerStatus {
    Disconnected,
    Connected,
    Error(String),
}

impl std::fmt::Display for McpServerStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disconnected => write!(formatter, "disconnected"),
            Self::Connected => write!(formatter, "connected"),
            Self::Error(error) => write!(formatter, "error: {error}"),
        }
    }
}

/// Tool definition exposed to callers after server-level filtering and policy
/// classification have been applied.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolDef {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub input_schema: Value,
    pub output_schema: Option<Value>,
    pub annotations: Option<Value>,
    pub approval: McpToolApproval,
    pub server_instructions: Option<String>,
}

impl McpToolDef {
    pub fn from_tool(tool: rmcp::model::Tool, approval: McpToolApproval) -> Self {
        Self {
            name: tool.name.to_string(),
            title: tool.title,
            description: tool.description.as_deref().map(ToString::to_string),
            input_schema: serde_json::to_value(&*tool.input_schema)
                .unwrap_or(Value::Object(serde_json::Map::new())),
            output_schema: tool
                .output_schema
                .as_deref()
                .and_then(|schema| serde_json::to_value(schema).ok()),
            annotations: tool
                .annotations
                .as_ref()
                .and_then(|annotations| serde_json::to_value(annotations).ok()),
            approval,
            server_instructions: None,
        }
    }
}

impl From<rmcp::model::Tool> for McpToolDef {
    fn from(tool: rmcp::model::Tool) -> Self {
        Self::from_tool(tool, McpToolApproval::Inherit)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolResult {
    pub content: Vec<McpContent>,
    pub structured_content: Option<Value>,
    pub is_error: bool,
    pub metadata: Option<Value>,
}

/// Lossless, serializable content returned by an MCP tool.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum McpContent {
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<Value>,
    },
    Image {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<Value>,
    },
    Audio {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<Value>,
    },
    Resource {
        uri: String,
        #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        blob: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<Value>,
    },
    ResourceLink {
        resource: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<Value>,
    },
}

impl std::fmt::Display for McpContent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text { text, .. } => write!(formatter, "{text}"),
            Self::Image { mime_type, .. } => write!(formatter, "[Image: {mime_type}]"),
            Self::Audio { mime_type, .. } => write!(formatter, "[Audio: {mime_type}]"),
            Self::Resource {
                uri, text, blob, ..
            } => {
                write!(formatter, "{uri}")?;
                if let Some(text) = text {
                    write!(formatter, "\n{text}")?;
                } else if blob.is_some() {
                    write!(formatter, "\n[Binary resource]")?;
                }
                Ok(())
            }
            Self::ResourceLink { resource, .. } => write!(formatter, "{resource}"),
        }
    }
}

pub fn format_mcp_result(result: &McpToolResult) -> String {
    let mut formatted = String::new();
    for (index, content) in result.content.iter().enumerate() {
        if index > 0 {
            formatted.push('\n');
        }
        formatted.push_str(&content.to_string());
    }
    if formatted.is_empty() {
        if let Some(structured) = &result.structured_content {
            formatted = structured.to_string();
        }
    }
    formatted
}

impl From<rmcp::model::CallToolResult> for McpToolResult {
    fn from(result: rmcp::model::CallToolResult) -> Self {
        use rmcp::model::{RawContent, ResourceContents};

        let content = result
            .content
            .into_iter()
            .map(|content| {
                let annotations = content
                    .annotations
                    .as_ref()
                    .and_then(|value| serde_json::to_value(value).ok());
                match content.raw {
                    RawContent::Text(text) => McpContent::Text {
                        text: text.text,
                        annotations,
                        metadata: text
                            .meta
                            .as_ref()
                            .and_then(|value| serde_json::to_value(value).ok()),
                    },
                    RawContent::Image(image) => McpContent::Image {
                        data: image.data,
                        mime_type: image.mime_type,
                        annotations,
                        metadata: image
                            .meta
                            .as_ref()
                            .and_then(|value| serde_json::to_value(value).ok()),
                    },
                    RawContent::Audio(audio) => McpContent::Audio {
                        data: audio.data,
                        mime_type: audio.mime_type,
                        annotations,
                    },
                    RawContent::Resource(embedded) => match embedded.resource {
                        ResourceContents::TextResourceContents {
                            uri,
                            mime_type,
                            text,
                            meta,
                        } => McpContent::Resource {
                            uri,
                            mime_type,
                            text: Some(text),
                            blob: None,
                            annotations,
                            metadata: meta
                                .as_ref()
                                .and_then(|value| serde_json::to_value(value).ok()),
                        },
                        ResourceContents::BlobResourceContents {
                            uri,
                            mime_type,
                            blob,
                            meta,
                        } => McpContent::Resource {
                            uri,
                            mime_type,
                            text: None,
                            blob: Some(blob),
                            annotations,
                            metadata: meta
                                .as_ref()
                                .and_then(|value| serde_json::to_value(value).ok()),
                        },
                    },
                    RawContent::ResourceLink(resource) => McpContent::ResourceLink {
                        resource: serde_json::to_value(resource).unwrap_or(Value::Null),
                        annotations,
                    },
                }
            })
            .collect();

        Self {
            content,
            structured_content: result.structured_content,
            is_error: result.is_error.unwrap_or(false),
            metadata: result
                .meta
                .as_ref()
                .and_then(|value| serde_json::to_value(value).ok()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct McpServerInfo {
    pub name: String,
    pub server_type: String,
    pub source: McpConfigSource,
    pub enabled: bool,
    pub required: bool,
    pub status: McpServerStatus,
    pub tool_count: usize,
    pub tools: Vec<McpToolDef>,
    pub instructions: Option<String>,
    pub server_info: Option<Value>,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::{CallToolResult, Content, RawContent};
    use serde_json::json;

    #[test]
    fn preserves_structured_and_binary_tool_content() {
        let mut result = CallToolResult::success(vec![Content::new(
            RawContent::image("aGVsbG8=", "image/png"),
            None,
        )]);
        result.structured_content = Some(json!({"answer": 42}));

        let converted = McpToolResult::from(result);
        assert_eq!(converted.structured_content, Some(json!({"answer": 42})));
        assert!(matches!(
            converted.content.as_slice(),
            [McpContent::Image { data, mime_type, .. }]
                if data == "aGVsbG8=" && mime_type == "image/png"
        ));
        let serialized = serde_json::to_value(converted).unwrap();
        assert_eq!(serialized["content"][0]["data"], "aGVsbG8=");
    }
}
