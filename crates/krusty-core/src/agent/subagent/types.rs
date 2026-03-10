//! Sub-agent types and data structures
//!
//! Core types for sub-agent configuration, progress tracking, and results.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;

use crate::ai::retry::is_retryable_status;
use crate::ai::retry::IsRetryable;
use crate::tools::registry::DelegationPolicy;

/// Error type for subagent API calls that supports retry logic
#[derive(Debug)]
pub struct SubAgentApiError {
    pub message: String,
    pub status: Option<u16>,
    pub retry_after: Option<Duration>,
}

impl std::fmt::Display for SubAgentApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(status) = self.status {
            write!(f, "HTTP {}: {}", status, self.message)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

impl std::error::Error for SubAgentApiError {}

impl IsRetryable for SubAgentApiError {
    fn is_retryable(&self) -> bool {
        match self.status {
            Some(status) => is_retryable_status(status),
            // Network errors without status codes are typically retryable
            None => {
                self.message.contains("timeout")
                    || self.message.contains("connection")
                    || self.message.contains("network")
            }
        }
    }

    fn retry_after(&self) -> Option<Duration> {
        self.retry_after
    }
}

impl From<anyhow::Error> for SubAgentApiError {
    fn from(err: anyhow::Error) -> Self {
        let message = err.to_string();
        // Try to extract HTTP status from error message
        let status = extract_status_from_error(&message);
        Self {
            message,
            status,
            retry_after: None,
        }
    }
}

/// Try to extract HTTP status code from error message
pub fn extract_status_from_error(message: &str) -> Option<u16> {
    // Common patterns: "HTTP 429", "status: 429", "status code: 429"
    for pattern in &["HTTP ", "status: ", "status code: "] {
        if let Some(pos) = message.find(pattern) {
            let start = pos + pattern.len();
            let code_str: String = message[start..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(code) = code_str.parse() {
                return Some(code);
            }
        }
    }
    None
}

/// Real-time progress update from a sub-agent
#[derive(Debug, Clone, Default)]
pub struct AgentProgress {
    /// Agent task ID
    pub task_id: String,
    /// Display name (derived from task context)
    pub name: String,
    /// Current status
    pub status: AgentProgressStatus,
    /// Number of tool calls made
    pub tool_count: usize,
    /// Approximate token usage
    pub tokens: usize,
    /// Current action description (e.g., "reading app.rs")
    pub current_action: Option<String>,
    /// Short completion summary when the sub-agent finishes a delegated task.
    pub completion_summary: Option<String>,
    /// Lines added (for build agents)
    pub lines_added: usize,
    /// Lines removed (for build agents)
    pub lines_removed: usize,
    /// Plan task ID completed (for auto-marking tasks)
    pub completed_plan_task: Option<String>,
}

/// Status of a sub-agent
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AgentProgressStatus {
    /// Agent is running
    #[default]
    Running,
    /// Agent completed successfully
    Complete,
    /// Agent failed
    Failed,
}

/// Configuration for a sub-agent task
///
/// The model to use is determined by `SubAgentPool.override_model`, not by the task.
/// This provides a provider-agnostic experience where all sub-agents use the user's
/// current model.
#[derive(Debug, Clone)]
pub struct SubAgentTask {
    pub id: String,
    /// Display name for the agent (e.g., "tui", "agent", "main")
    pub name: String,
    pub prompt: String,
    pub working_dir: PathBuf,
    /// Plan task ID this agent completes (for auto-marking)
    pub plan_task_id: Option<String>,
    /// Whether thinking/reasoning is enabled for this agent
    pub thinking_enabled: bool,
    /// Inherited delegated execution policy from parent tool context.
    pub delegation_policy: Option<DelegationPolicy>,
    /// Optional per-task turn budget inherited from parent.
    pub max_turns_override: Option<usize>,
}

impl SubAgentTask {
    pub fn new(id: impl Into<String>, prompt: impl Into<String>) -> Self {
        let id = id.into();
        let name = id.clone(); // Default name is same as id
        Self {
            id,
            name,
            prompt: prompt.into(),
            working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            plan_task_id: None,
            thinking_enabled: false, // Default off for sub-agents
            delegation_policy: None,
            max_turns_override: None,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn with_working_dir(mut self, dir: PathBuf) -> Self {
        self.working_dir = dir;
        self
    }

    pub fn with_plan_task_id(mut self, task_id: impl Into<String>) -> Self {
        self.plan_task_id = Some(task_id.into());
        self
    }

    pub fn with_thinking(mut self, enabled: bool) -> Self {
        self.thinking_enabled = enabled;
        self
    }

    pub fn with_delegation_policy(mut self, policy: DelegationPolicy) -> Self {
        self.delegation_policy = Some(policy);
        self
    }

    pub fn with_max_turns(mut self, max_turns: usize) -> Self {
        self.max_turns_override = Some(max_turns);
        self
    }

    pub(crate) fn system_prompt(&self) -> String {
        format!(
            r#"You are a codebase explorer. Your task is to systematically investigate the codebase and answer questions.

## Working Directory
{}

## Available Tools
You have read-only access to these tools - USE THEM:

1. **glob** - Find files by pattern
   - Start here to discover file structure
   - Examples: `**/*.rs`, `src/**/*.ts`, `**/test*`

2. **grep** - Search file contents with regex
   - Find specific patterns, functions, or keywords
   - Use after glob to narrow down relevant files

3. **read** - Read file contents
   - Read specific files to understand implementation details
   - Always read files you need to answer questions about

## Instructions
1. START by using glob to find relevant files in the directory
2. Use grep to search for specific patterns or keywords
3. Read the most relevant files to understand the code
4. Be THOROUGH - examine multiple files, not just one
5. Track what files you examine and report them in your summary

## Output Format
When you have gathered enough information, provide:
1. A clear answer to the question
2. List of key files examined
3. Specific code references where relevant

Do NOT skip tool usage - always explore before answering."#,
            self.working_dir.display()
        )
    }
}

/// Result from a sub-agent execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentResult {
    pub task_id: String,
    pub success: bool,
    pub output: String,
    pub files_examined: Vec<String>,
    pub duration_ms: u64,
    pub turns_used: usize,
    pub error: Option<String>,
    pub policy_violations: Vec<String>,
}

impl SubAgentResult {
    pub fn brief_summary(&self) -> String {
        let lines = self
            .output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .take(4)
            .collect::<Vec<_>>();

        let summary = if lines.is_empty() {
            self.error
                .clone()
                .unwrap_or_else(|| "No summary produced".to_string())
        } else {
            lines.join(" ")
        };

        truncate_preview(&summary, 600)
    }

    pub fn evidence_json(&self) -> Value {
        json!({
            "agent": self.task_id,
            "success": self.success,
            "summary": self.brief_summary(),
            "files_examined": dedup_files(&self.files_examined, 12),
            "turns_used": self.turns_used,
            "duration_ms": self.duration_ms,
            "error": self.error,
            "policy_violations": self.policy_violations,
        })
    }
}

fn dedup_files(files: &[String], limit: usize) -> Vec<String> {
    let mut unique = Vec::new();
    for file in files {
        if !unique.iter().any(|existing| existing == file) {
            unique.push(file.clone());
        }
        if unique.len() >= limit {
            break;
        }
    }
    unique
}

fn truncate_preview(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }

    let mut boundary = max_chars.min(text.len());
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }

    text[..boundary].trim_end().to_string()
}

/// Parsed tool call from API response
#[derive(Debug)]
pub(crate) struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}
