use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::tools::registry::Tool;
use crate::tools::{ToolContext, ToolResult};

use super::process::AgentExtensionProcess;
use super::ExtensionCallContext;

pub(crate) struct AgentExtensionTool {
    public_name: String,
    runtime_name: String,
    extension_id: String,
    description: String,
    parameters: Value,
    process: Arc<Mutex<AgentExtensionProcess>>,
}

impl AgentExtensionTool {
    pub fn new(
        public_name: String,
        runtime_name: String,
        extension_id: String,
        description: String,
        parameters: Value,
        process: Arc<Mutex<AgentExtensionProcess>>,
    ) -> Self {
        Self {
            public_name,
            runtime_name,
            extension_id,
            description,
            parameters,
            process,
        }
    }
}

#[async_trait]
impl Tool for AgentExtensionTool {
    fn name(&self) -> &str {
        &self.public_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        self.parameters.clone()
    }

    async fn execute(&self, params: Value, context: &ToolContext) -> ToolResult {
        let extension_context = ExtensionCallContext::from_tool_context(context);
        match self
            .process
            .lock()
            .await
            .call_tool(&self.runtime_name, params, &extension_context)
            .await
        {
            Ok(Value::String(output)) => ToolResult::success(output),
            Ok(output) => ToolResult::success_data(output),
            Err(error) => ToolResult::error_with_details(
                "agent_extension_error",
                error,
                None,
                Some(serde_json::json!({
                    "extension_id": self.extension_id,
                    "tool": self.runtime_name,
                })),
            ),
        }
    }
}
