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

    /// Attach a producer-owned semantic state delta to this result.
    pub fn with_changed(mut self, changed: bool) -> Self {
        let mut envelope = serde_json::from_str::<Value>(&self.output)
            .ok()
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_else(|| {
                let mut object = serde_json::Map::new();
                object.insert("ok".to_string(), Value::Bool(!self.is_error));
                object.insert("data".to_string(), Value::String(self.output.clone()));
                object
            });
        envelope.insert("changed".to_string(), Value::Bool(changed));
        self.output = Value::Object(envelope).to_string();
        self
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

/// Parse tool parameters, returning a ToolResult error on failure.
///
/// Some OpenAI-compatible providers (notably the Grok CLI proxy) emit
/// integer-looking tool arguments as JSON floats, e.g. `{ "depth": 2.0 }`.
/// Rust tool params often use `usize`/`u64`, so normalize integral floats before
/// deserializing while leaving true fractional values untouched.
pub fn parse_params<T: serde::de::DeserializeOwned>(mut params: Value) -> Result<T, ToolResult> {
    normalize_integral_floats(&mut params);
    serde_json::from_value(params)
        .map_err(|e| ToolResult::invalid_parameters(format!("Invalid parameters: {}", e)))
}

fn normalize_integral_floats(value: &mut Value) {
    match value {
        Value::Number(number) if number.as_i64().is_none() && number.as_u64().is_none() => {
            let Some(float) = number.as_f64() else {
                return;
            };
            if !float.is_finite() || float.fract() != 0.0 {
                return;
            }

            if float >= i64::MIN as f64 && float <= i64::MAX as f64 {
                *value = Value::Number(serde_json::Number::from(float as i64));
            } else if float >= 0.0 && float <= u64::MAX as f64 {
                *value = Value::Number(serde_json::Number::from(float as u64));
            }
        }
        Value::Array(items) => {
            for item in items {
                normalize_integral_floats(item);
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                normalize_integral_floats(item);
            }
        }
        _ => {}
    }
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
