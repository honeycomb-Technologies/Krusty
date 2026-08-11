//! Typed Codex app-server MCP resource-reading contracts.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpResourceReadParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub server: String,
    pub uri: String,
}

impl McpResourceReadParams {
    pub fn new(server: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            thread_id: None,
            server: server.into(),
            uri: uri.into(),
        }
    }
}

/// Text or binary contents returned by an MCP resource.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpResourceContent {
    Text {
        uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "mimeType")]
        mime_type: Option<String>,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "_meta")]
        meta: Option<Value>,
    },
    Blob {
        uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "mimeType")]
        mime_type: Option<String>,
        blob: String,
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "_meta")]
        meta: Option<Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpResourceReadResponse {
    pub contents: Vec<McpResourceContent>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_response_preserves_text_blob_and_metadata() {
        let response: McpResourceReadResponse = serde_json::from_value(serde_json::json!({
            "contents": [
                {"uri": "docs://one", "mimeType": "text/plain", "text": "hello"},
                {"uri": "asset://two", "blob": "AQI=", "_meta": {"source": "live"}}
            ]
        }))
        .unwrap();
        assert!(matches!(
            response.contents[0],
            McpResourceContent::Text { .. }
        ));
        assert!(matches!(
            response.contents[1],
            McpResourceContent::Blob { .. }
        ));
    }
}
