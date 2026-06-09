use serde_json::Value;

/// Tool execution result
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub output: String,
    pub is_error: bool,
}

impl ToolResult {
    /// Create a success result
    pub fn success(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            is_error: false,
        }
    }

    /// Create a structured success envelope with `ok=true` and `data`.
    pub fn success_data(data: Value) -> Self {
        Self::success_data_with(data, Vec::new(), None, None)
    }

    /// Create a structured success envelope with optional warnings/diff/metadata.
    pub fn success_data_with(
        data: Value,
        warnings: Vec<String>,
        diff: Option<String>,
        metadata: Option<Value>,
    ) -> Self {
        let mut envelope = serde_json::Map::new();
        envelope.insert("ok".to_string(), Value::Bool(true));
        envelope.insert("data".to_string(), data);

        if !warnings.is_empty() {
            envelope.insert(
                "warnings".to_string(),
                Value::Array(warnings.into_iter().map(Value::String).collect()),
            );
        }

        if let Some(diff) = diff.filter(|d| !d.is_empty()) {
            envelope.insert("diff".to_string(), Value::String(diff));
        }

        if let Some(metadata) = metadata {
            envelope.insert("metadata".to_string(), metadata);
        }

        Self {
            output: Value::Object(envelope).to_string(),
            is_error: false,
        }
    }

    /// Create a structured error with explicit code.
    pub fn error_with_code(code: &str, msg: impl std::fmt::Display) -> Self {
        Self::error_with_details(code, msg, None, None)
    }

    /// Create a structured error envelope with optional data/metadata.
    pub fn error_with_details(
        code: &str,
        msg: impl std::fmt::Display,
        data: Option<Value>,
        metadata: Option<Value>,
    ) -> Self {
        let mut envelope = serde_json::Map::new();
        envelope.insert("ok".to_string(), Value::Bool(false));
        envelope.insert(
            "error".to_string(),
            serde_json::json!({
                "code": code,
                "message": msg.to_string()
            }),
        );

        if let Some(data) = data {
            envelope.insert("data".to_string(), data);
        }

        if let Some(metadata) = metadata {
            envelope.insert("metadata".to_string(), metadata);
        }

        Self {
            output: Value::Object(envelope).to_string(),
            is_error: true,
        }
    }

    /// Create an invalid-parameters error.
    pub fn invalid_parameters(msg: impl std::fmt::Display) -> Self {
        Self::error_with_code("invalid_parameters", msg)
    }

    /// Create an error result with JSON-formatted error message
    pub fn error(msg: impl std::fmt::Display) -> Self {
        let message = msg.to_string();
        let code = classify_error_code(&message);
        Self::error_with_details(code, message, None, None)
    }
}

/// Parse tool parameters, returning a ToolResult error on failure
pub fn parse_params<T: serde::de::DeserializeOwned>(params: Value) -> Result<T, ToolResult> {
    serde_json::from_value(params)
        .map_err(|e| ToolResult::invalid_parameters(format!("Invalid parameters: {}", e)))
}

fn classify_error_code(message: &str) -> &'static str {
    let lower = message.to_ascii_lowercase();
    if lower.contains("invalid parameters")
        || lower.contains("missing field")
        || lower.contains("unknown field")
    {
        "invalid_parameters"
    } else if lower.contains("access denied")
        || lower.contains("outside workspace")
        || lower.contains("filesystem access root")
    {
        "access_denied"
    } else if lower.contains("timed out") || lower.contains("timeout") {
        "timeout"
    } else if lower.contains("denied") {
        "permission_denied"
    } else if lower.contains("unknown tool") {
        "unknown_tool"
    } else {
        "tool_error"
    }
}
