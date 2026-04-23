use serde_json::Value;

use crate::ai::types::AiToolCall;

/// Tool call accumulator for providers that stream tool calls in parts
#[derive(Debug, Clone)]
pub struct ToolCallAccumulator {
    pub id: String,
    pub name: String,
    pub arguments: String,
    pub is_complete: bool,
}

/// Server tool accumulator for web_search/web_fetch
#[derive(Debug, Clone)]
pub struct ServerToolAccumulator {
    pub id: String,
    pub name: String,
    pub input_json: String,
    pub is_complete: bool,
}

impl ServerToolAccumulator {
    pub fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            input_json: String::new(),
            is_complete: false,
        }
    }

    pub fn add_input(&mut self, delta: &str) {
        self.input_json.push_str(delta);
    }

    pub fn complete(&mut self) -> Value {
        self.is_complete = true;
        if self.input_json.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str::<Value>(&self.input_json)
                .unwrap_or_else(|_| serde_json::json!({"raw": self.input_json.clone()}))
        }
    }
}

/// Thinking block accumulator for extended thinking
#[derive(Debug, Clone)]
pub struct ThinkingAccumulator {
    pub thinking: String,
    pub signature: String,
    pub is_complete: bool,
}

impl ThinkingAccumulator {
    pub fn new() -> Self {
        Self {
            thinking: String::new(),
            signature: String::new(),
            is_complete: false,
        }
    }

    pub fn add_thinking(&mut self, delta: &str) {
        self.thinking.push_str(delta);
    }

    pub fn add_signature(&mut self, delta: &str) {
        self.signature.push_str(delta);
    }

    pub fn complete(&mut self) -> (String, String) {
        self.is_complete = true;
        (self.thinking.clone(), self.signature.clone())
    }
}

impl Default for ThinkingAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolCallAccumulator {
    pub fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            arguments: String::new(),
            is_complete: false,
        }
    }

    pub fn add_arguments(&mut self, delta: &str) {
        self.arguments.push_str(delta);
    }

    pub fn try_complete(&mut self) -> Option<AiToolCall> {
        if !self.arguments.is_empty() {
            if let Ok(parsed) = serde_json::from_str::<Value>(&self.arguments) {
                self.is_complete = true;
                return Some(AiToolCall {
                    id: self.id.clone(),
                    name: self.name.clone(),
                    arguments: parsed,
                });
            }
        }
        None
    }

    pub fn force_complete(&mut self) -> AiToolCall {
        self.is_complete = true;
        AiToolCall {
            id: self.id.clone(),
            name: self.name.clone(),
            arguments: if self.arguments.is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str::<Value>(&self.arguments)
                    .unwrap_or_else(|_| serde_json::json!({"raw": self.arguments.clone()}))
            },
        }
    }
}
