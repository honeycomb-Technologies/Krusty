use serde_json::Value;
use sha2::{Digest, Sha256};

/// Read producer-owned semantic change evidence from the result envelope.
///
/// Only the root field is trusted. Nested tool payloads are arbitrary data and
/// must not be able to spoof orchestration progress or validation state.
pub fn trusted_changed(value: &Value) -> Option<bool> {
    match value {
        Value::Object(object) => object.get("changed").and_then(Value::as_bool),
        Value::String(serialized) => serde_json::from_str::<Value>(serialized)
            .ok()
            .as_ref()
            .and_then(trusted_changed),
        _ => None,
    }
}

/// Hash a canonical set of workspace resources for semantic progress
/// accounting. Payload bytes are deliberately excluded: repeatedly rewriting
/// one target is one effect intent even when the generated content changes.
pub fn progress_change_key_for_paths(
    paths: &[std::path::PathBuf],
    scope_root: &std::path::Path,
) -> String {
    let canonical_root = scope_root
        .canonicalize()
        .unwrap_or_else(|_| scope_root.to_path_buf());
    let mut resources = paths
        .iter()
        .map(|path| {
            path.strip_prefix(&canonical_root)
                .or_else(|_| path.strip_prefix(scope_root))
                .unwrap_or(path)
        })
        .map(|path| path.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    resources.sort();
    resources.dedup();

    let mut hasher = Sha256::new();
    hasher.update(b"workspace-path-set-v1\0");
    for resource in resources {
        hasher.update(resource.as_bytes());
        hasher.update([0]);
    }
    format!("{:x}", hasher.finalize())
}

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

    /// Create a machine-actionable error. `retryable` describes repeating the
    /// same operation, not whether the overall objective can recover through a
    /// different action.
    pub fn error_with_recovery(
        code: &str,
        msg: impl std::fmt::Display,
        retryable: bool,
        next_action: impl Into<String>,
        prohibited_retries: Vec<String>,
        safe_alternative: Option<Value>,
    ) -> Self {
        let mut error = serde_json::Map::new();
        error.insert("code".to_string(), Value::String(code.to_string()));
        error.insert("message".to_string(), Value::String(msg.to_string()));
        error.insert("retryable".to_string(), Value::Bool(retryable));
        error.insert("next_action".to_string(), Value::String(next_action.into()));
        if !prohibited_retries.is_empty() {
            error.insert(
                "prohibited_retries".to_string(),
                Value::Array(prohibited_retries.into_iter().map(Value::String).collect()),
            );
        }
        if let Some(safe_alternative) = safe_alternative {
            error.insert("safe_alternative".to_string(), safe_alternative);
        }

        Self {
            output: serde_json::json!({
                "ok": false,
                "error": Value::Object(error),
            })
            .to_string(),
            is_error: true,
        }
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

    /// Attach a producer-owned, already-hashed identity for the state surface
    /// that changed. The progress ledger uses this instead of attempting to
    /// infer shell semantics from command text.
    pub fn with_progress_change_key(mut self, key: impl Into<String>) -> Self {
        let mut envelope = serde_json::from_str::<Value>(&self.output)
            .ok()
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_else(|| {
                let mut object = serde_json::Map::new();
                object.insert("ok".to_string(), Value::Bool(!self.is_error));
                object.insert("data".to_string(), Value::String(self.output.clone()));
                object
            });
        let metadata = envelope
            .entry("metadata".to_string())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if !metadata.is_object() {
            *metadata = Value::Object(serde_json::Map::new());
        }
        if let Some(metadata) = metadata.as_object_mut() {
            metadata.insert("progress_change_key".to_string(), Value::String(key.into()));
        }
        self.output = Value::Object(envelope).to_string();
        self
    }

    pub fn with_progress_change_paths(
        self,
        paths: &[std::path::PathBuf],
        scope_root: &std::path::Path,
    ) -> Self {
        self.with_progress_change_key(progress_change_key_for_paths(paths, scope_root))
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
