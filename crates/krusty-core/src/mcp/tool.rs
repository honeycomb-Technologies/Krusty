//! MCP Tool wrapper
//!
//! Wraps MCP tools as our Tool trait for seamless integration.
//!
//! NOTE: MCP tools execute on external servers and bypass Krusty's sandbox.
//! When sandbox_root is configured, a warning is logged for visibility.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::warn;

use super::manager::McpManager;
use super::protocol::{format_mcp_result, McpToolDef};
use crate::tools::registry::{Tool, ToolContext, ToolResult};

fn sanitize_schema(schema: &Value) -> Value {
    let mut normalized = match schema {
        Value::Object(_) => schema.clone(),
        _ => json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
    };

    sanitize_schema_in_place(&mut normalized);
    normalized
}

fn sanitize_schema_in_place(schema: &mut Value) {
    let Value::Object(map) = schema else {
        return;
    };

    if let Some(Value::Object(properties)) = map.get_mut("properties") {
        for value in properties.values_mut() {
            sanitize_schema_in_place(value);
        }
    }

    if let Some(items) = map.get_mut("items") {
        sanitize_schema_in_place(items);
    }

    for key in ["allOf", "anyOf", "oneOf"] {
        if let Some(Value::Array(items)) = map.get_mut(key) {
            for item in items.iter_mut() {
                sanitize_schema_in_place(item);
            }
        }
    }

    let declared_type = map.get("type").and_then(|v| v.as_str());
    let has_object_shape = map.get("properties").is_some();
    let is_object =
        declared_type == Some("object") || (declared_type.is_none() && has_object_shape);
    if !is_object {
        return;
    }

    if !matches!(map.get("properties"), Some(Value::Object(_))) {
        map.insert("properties".to_string(), json!({}));
    }

    if !matches!(
        map.get("additionalProperties"),
        Some(Value::Bool(_)) | Some(Value::Object(_))
    ) {
        map.insert("additionalProperties".to_string(), Value::Bool(false));
    }

    let remove_required = match map.get_mut("required") {
        Some(Value::Array(entries)) => {
            entries.retain(|v| v.is_string());
            false
        }
        Some(_) => true,
        None => false,
    };
    if remove_required {
        map.remove("required");
    }
}

/// Wraps an MCP tool as our Tool trait
pub struct McpTool {
    server_name: String,
    tool_name: String,
    full_name: String,
    definition: McpToolDef,
    manager: Arc<McpManager>,
}

impl McpTool {
    pub fn new(server_name: String, mut definition: McpToolDef, manager: Arc<McpManager>) -> Self {
        definition.input_schema = sanitize_schema(&definition.input_schema);
        let tool_name = definition.name.clone();
        let full_name = format!("mcp__{}_{}", server_name, tool_name);

        Self {
            server_name,
            tool_name,
            full_name,
            definition,
            manager,
        }
    }
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.full_name
    }

    fn description(&self) -> &str {
        self.definition.description.as_deref().unwrap_or("MCP tool")
    }

    fn parameters_schema(&self) -> Value {
        self.definition.input_schema.clone()
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let sandbox_warning =
            "MCP tool bypasses local sandbox restrictions because it executes on an external server.";

        // Warn when MCP tools are used in sandboxed mode - they bypass sandbox restrictions
        if ctx.sandbox_root.is_some() {
            warn!(
                "MCP tool '{}' executing in sandboxed context - MCP servers bypass sandbox restrictions",
                self.full_name
            );
        }

        match self
            .manager
            .call_tool(&self.server_name, &self.tool_name, params)
            .await
        {
            Ok(result) => {
                let output = format_mcp_result(&result);
                let metadata = Some(json!({
                    "server": self.server_name.clone(),
                    "tool": self.tool_name.clone(),
                    "is_remote_execution": true,
                    "content_items": result.content.len()
                }));
                let warnings = if ctx.sandbox_root.is_some() {
                    vec![sandbox_warning.to_string()]
                } else {
                    Vec::new()
                };

                if result.is_error {
                    ToolResult::error_with_details(
                        "mcp_tool_error",
                        "MCP server returned an error result",
                        Some(json!({ "output": output })),
                        metadata,
                    )
                } else {
                    ToolResult::success_data_with(
                        json!({ "output": output }),
                        warnings,
                        None,
                        metadata,
                    )
                }
            }
            Err(e) => ToolResult::error_with_details(
                "mcp_call_failed",
                format!("MCP error: {}", e),
                None,
                Some(json!({
                    "server": self.server_name.clone(),
                    "tool": self.tool_name.clone(),
                    "is_remote_execution": true
                })),
            ),
        }
    }
}

/// Register all MCP tools from connected servers
pub async fn register_mcp_tools(manager: Arc<McpManager>, registry: &crate::tools::ToolRegistry) {
    let tools = manager.get_all_tools().await;

    for (server_name, tool_def) in tools {
        let mcp_tool = Arc::new(McpTool::new(server_name, tool_def, manager.clone()));
        registry.register(mcp_tool).await;
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_schema;
    use serde_json::json;

    #[test]
    fn sanitize_schema_adds_object_defaults() {
        let schema = json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" }
            }
        });
        let sanitized = sanitize_schema(&schema);

        assert_eq!(sanitized["type"], "object");
        assert_eq!(sanitized["additionalProperties"], false);
        assert!(sanitized["properties"].is_object());
    }

    #[test]
    fn sanitize_schema_replaces_non_object_root() {
        let schema = json!("not-a-schema");
        let sanitized = sanitize_schema(&schema);

        assert_eq!(sanitized["type"], "object");
        assert!(sanitized["properties"].is_object());
        assert_eq!(sanitized["additionalProperties"], false);
    }

    #[test]
    fn sanitize_schema_filters_invalid_required_entries() {
        let schema = json!({
            "type": "object",
            "properties": { "a": { "type": "string" } },
            "required": ["a", 123, null]
        });
        let sanitized = sanitize_schema(&schema);
        let required = sanitized["required"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert_eq!(required, vec![json!("a")]);
    }
}
