//! Little Claw - The Analyst Agent
//!
//! Questions everything, ensures quality, but cannot modify files.
//! Has read-only tools for research: Read, Glob, Grep.

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};

use super::dialogue::{DialogueResult, DialogueTurn, Speaker};
use super::observation::{Observation, ObservationLog};
use super::roles::ClawRole;
use crate::ai::client::config::CallOptions;
use crate::ai::client::core::KRUSTY_SYSTEM_PROMPT;
use crate::ai::client::AiClient;
use crate::ai::streaming::StreamPart;
use crate::ai::types::{AiTool, AiToolCall, Content, ModelMessage, Role};
use crate::tools::{ToolContext, ToolRegistry, ToolResult as ToolExecResult};

/// Maximum agentic turns for Little Claw (prevent runaway research)
const MAX_RESEARCH_TURNS: usize = 3;

/// Little Claw - the analyst that questions everything
pub struct LittleClaw {
    /// AI client for making requests
    client: Arc<AiClient>,

    /// Tool registry for executing research tools
    tools: Option<Arc<ToolRegistry>>,

    /// Working directory for tool execution
    working_dir: PathBuf,

    /// Independent conversation history
    messages: Arc<RwLock<Vec<ModelMessage>>>,

    /// Log of Big Claw's actions (for context sync)
    observation_log: Arc<RwLock<ObservationLog>>,

    /// How many observations we've already processed
    observation_cursor: Arc<RwLock<usize>>,
}

impl LittleClaw {
    pub fn new(client: Arc<AiClient>) -> Self {
        Self {
            client,
            tools: None,
            working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            messages: Arc::new(RwLock::new(Vec::new())),
            observation_log: Arc::new(RwLock::new(ObservationLog::new())),
            observation_cursor: Arc::new(RwLock::new(0)),
        }
    }

    /// Set the tool registry for research capabilities
    pub fn with_tools(mut self, tools: Arc<ToolRegistry>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Set the working directory
    pub fn with_working_dir(mut self, dir: PathBuf) -> Self {
        self.working_dir = dir;
        self
    }

    /// Build the system prompt (base + role layer)
    fn system_prompt() -> String {
        format!(
            "{}\n\n---\n\n{}",
            KRUSTY_SYSTEM_PROMPT,
            ClawRole::LittleClaw.prompt_layer()
        )
    }

    /// Build CallOptions for Little Claw
    fn call_options() -> CallOptions {
        CallOptions {
            system_prompt: Some(Self::system_prompt()),
            tools: Some(Self::tools_schema()),
            max_tokens: Some(2048), // Little Claw should be concise
            enable_caching: true,
            ..Default::default()
        }
    }

    /// Get the tool schemas available to Little Claw (read-only research)
    fn tools_schema() -> Vec<AiTool> {
        vec![
            AiTool {
                name: "Read".to_string(),
                description: "Read a file to understand existing patterns and code".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "file_path": {
                            "type": "string",
                            "description": "Absolute path to the file to read"
                        }
                    },
                    "required": ["file_path"]
                }),
            },
            AiTool {
                name: "Grep".to_string(),
                description: "Search for patterns in the codebase".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "pattern": {
                            "type": "string",
                            "description": "Regex pattern to search for"
                        },
                        "path": {
                            "type": "string",
                            "description": "Directory to search in (defaults to current)"
                        }
                    },
                    "required": ["pattern"]
                }),
            },
            AiTool {
                name: "Glob".to_string(),
                description: "Find files matching a glob pattern".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "pattern": {
                            "type": "string",
                            "description": "Glob pattern (e.g., **/*.rs)"
                        }
                    },
                    "required": ["pattern"]
                }),
            },
        ]
    }

    /// Record an observation from Big Claw's action
    pub async fn observe(&self, observation: Observation) {
        let mut log = self.observation_log.write().await;
        log.record(observation);
    }

    /// Sync new observations into our context
    async fn sync_observations(&self) {
        let log = self.observation_log.read().await;
        let mut cursor = self.observation_cursor.write().await;

        let new_observations = log.since(*cursor);
        if new_observations.is_empty() {
            return;
        }

        // Build context message from new observations
        let context: String = new_observations
            .iter()
            .map(|o| o.as_context_message())
            .collect::<Vec<_>>()
            .join("\n\n");

        // Add to our message history as user message (context update)
        let mut messages = self.messages.write().await;
        messages.push(ModelMessage {
            role: Role::User,
            content: vec![Content::Text {
                text: format!("[Context Update - Big Claw's Actions]\n{}", context),
            }],
        });

        // Update cursor
        *cursor = log.len();

        debug!(
            new_observations = new_observations.len(),
            "Little Claw synced observations"
        );
    }

    /// Review Big Claw's stated intent before action
    pub async fn review_intent(&self, intent: &str) -> DialogueResult {
        // Sync any pending observations first
        self.sync_observations().await;

        // DEBUG: Log to file for verification
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/dual_mind_dialogue.log")
        {
            use std::io::Write;
            let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
            let _ = writeln!(file, "\n[{}] === PRE-REVIEW ===", timestamp);
            let _ = writeln!(file, "[BIG CLAW INTENT]: {}", intent);
        }

        let prompt = format!(
            "Big Claw is about to take this action:\n\n{}\n\n\
            Review this intent. Ask yourself:\n\
            - Is this the simplest approach?\n\
            - Does this match existing patterns?\n\
            - Are we over-engineering?\n\
            - What could go wrong?\n\n\
            If you need to check existing code patterns, use the Read or Grep tools.\n\
            If trivial and correct, just say \"Proceed.\"\n\
            If you have concerns, state them clearly and specifically.\n\
            Keep your response concise - one or two sentences.",
            intent
        );

        self.generate_response(&prompt).await
    }

    /// Review Big Claw's output after action
    pub async fn review_output(&self, output: &str) -> DialogueResult {
        // Sync any pending observations first
        self.sync_observations().await;

        // DEBUG: Log to file for verification
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/dual_mind_dialogue.log")
        {
            use std::io::Write;
            let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
            let _ = writeln!(file, "\n[{}] === POST-REVIEW ===", timestamp);
            let _ = writeln!(
                file,
                "[BIG CLAW OUTPUT]: {}...",
                &output.chars().take(500).collect::<String>()
            );
        }

        // Truncate very long output (char-boundary safe)
        let truncated_output = if output.len() > 3000 {
            let mut end = 3000;
            while end > 0 && !output.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}...[truncated]", &output[..end])
        } else {
            output.to_string()
        };

        let prompt = format!(
            "Big Claw just produced this output:\n\n```\n{}\n```\n\n\
            Review this output. Check:\n\
            - Does it match the stated intent?\n\
            - Is the code elegant and idiomatic?\n\
            - Any obvious issues or inconsistencies?\n\n\
            If you notice reusable patterns or conventions, state them as general rules \
            (e.g., \"This codebase consistently uses X pattern for Y\").\n\n\
            If you need to verify against existing patterns, use Read or Grep.\n\
            If good, say \"Approved.\"\n\
            If issues found, briefly state what needs enhancement.\n\
            Keep your response concise.",
            truncated_output
        );

        self.generate_response(&prompt).await
    }

    /// Generate a response using the AI client with agentic tool loop
    async fn generate_response(&self, prompt: &str) -> DialogueResult {
        // Add the prompt to our history
        {
            let mut messages = self.messages.write().await;
            messages.push(ModelMessage {
                role: Role::User,
                content: vec![Content::Text {
                    text: prompt.to_string(),
                }],
            });
        }

        // Agentic loop - Little Claw might need to research
        for turn in 0..MAX_RESEARCH_TURNS {
            // Get current messages for API call
            let messages = self.messages.read().await.clone();
            let options = Self::call_options();

            info!(
                messages = messages.len(),
                turn = turn,
                "Little Claw making API call"
            );

            // Make the streaming API call
            // DEBUG: Log API call attempt
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/dual_mind_dialogue.log")
            {
                use std::io::Write;
                let _ = writeln!(file, "[LITTLE CLAW] Making API call (turn {})...", turn);
            }

            let rx = match self.client.call_streaming(messages, &options).await {
                Ok(rx) => rx,
                Err(e) => {
                    warn!("Little Claw API call failed: {}", e);
                    // DEBUG: Log failure
                    if let Ok(mut file) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open("/tmp/dual_mind_dialogue.log")
                    {
                        use std::io::Write;
                        let _ = writeln!(file, "[LITTLE CLAW] API CALL FAILED: {}", e);
                    }
                    return DialogueResult::Skipped;
                }
            };

            // Consume the stream
            let (response, tool_calls) = self.consume_stream(rx).await;

            // If we got tool calls, execute them and continue
            if !tool_calls.is_empty() {
                // Add assistant message with tool calls to history
                self.add_assistant_with_tools(&response, &tool_calls).await;

                // Execute tools and add results
                self.execute_tool_calls(&tool_calls).await;

                // Continue loop for next turn
                continue;
            }

            // No tool calls - we have our final response
            if response.is_empty() {
                warn!("Little Claw received empty response");
                return DialogueResult::Skipped;
            }

            // Create dialogue turn for result
            let turn = DialogueTurn {
                speaker: Speaker::LittleClaw,
                content: response.clone(),
            };

            // Add to our history
            {
                let mut messages = self.messages.write().await;
                messages.push(ModelMessage {
                    role: Role::Assistant,
                    content: vec![Content::Text {
                        text: response.clone(),
                    }],
                });
            }

            // DEBUG: Log Little Claw's response
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/dual_mind_dialogue.log")
            {
                use std::io::Write;
                let _ = writeln!(file, "[LITTLE CLAW RESPONSE]: {}", response);
                let _ = writeln!(file, "---");
            }

            // Prune messages if history is too long
            self.prune_messages().await;

            // Determine result type based on response content
            return self.classify_response(&response, turn);
        }

        // Hit max turns - return what we have
        warn!("Little Claw hit max research turns");
        DialogueResult::Skipped
    }

    /// Consume streaming response and return text + tool calls
    async fn consume_stream(
        &self,
        mut rx: mpsc::UnboundedReceiver<StreamPart>,
    ) -> (String, Vec<AiToolCall>) {
        let mut full_text = String::new();
        let mut tool_calls: Vec<AiToolCall> = Vec::new();

        while let Some(part) = rx.recv().await {
            match part {
                StreamPart::TextDelta { delta } => {
                    full_text.push_str(&delta);
                }
                StreamPart::TextDeltaWithCitations { delta, .. } => {
                    full_text.push_str(&delta);
                }
                StreamPart::ToolCallComplete { tool_call } => {
                    debug!(tool = %tool_call.name, "Little Claw tool call");
                    tool_calls.push(tool_call);
                }
                StreamPart::Finish { .. } => {
                    debug!("Little Claw stream finished");
                    break;
                }
                _ => {}
            }
        }

        (full_text.trim().to_string(), tool_calls)
    }

    /// Add assistant message with tool calls to history
    async fn add_assistant_with_tools(&self, text: &str, tool_calls: &[AiToolCall]) {
        let mut content = Vec::new();

        if !text.is_empty() {
            content.push(Content::Text {
                text: text.to_string(),
            });
        }

        for tc in tool_calls {
            content.push(Content::ToolUse {
                id: tc.id.clone(),
                name: tc.name.clone(),
                input: tc.arguments.clone(),
            });
        }

        let mut messages = self.messages.write().await;
        messages.push(ModelMessage {
            role: Role::Assistant,
            content,
        });
    }

    /// Execute tool calls and add results to history
    /// IMPORTANT: All tool results must be added in a SINGLE user message
    async fn execute_tool_calls(&self, tool_calls: &[AiToolCall]) {
        let mut results: Vec<Content> = Vec::new();

        let Some(tools) = &self.tools else {
            // No tools available - add error results
            for tc in tool_calls {
                results.push(Content::ToolResult {
                    tool_use_id: tc.id.clone(),
                    output: serde_json::Value::String("Tools not available".to_string()),
                    is_error: Some(true),
                });
            }
            self.add_tool_results(results).await;
            return;
        };

        let ctx = ToolContext {
            working_dir: self.working_dir.clone(),
            ..Default::default()
        };

        for tc in tool_calls {
            // Only allow read-only tools
            if !matches!(tc.name.as_str(), "Read" | "Grep" | "Glob") {
                results.push(Content::ToolResult {
                    tool_use_id: tc.id.clone(),
                    output: serde_json::Value::String(format!(
                        "Tool '{}' not allowed for Little Claw",
                        tc.name
                    )),
                    is_error: Some(true),
                });
                continue;
            }

            info!(tool = %tc.name, "Little Claw executing research tool");

            let result = tools.execute(&tc.name, tc.arguments.clone(), &ctx).await;

            match result {
                Some(ToolExecResult { output, is_error }) => {
                    // Truncate very long outputs (char-boundary safe)
                    let truncated = if output.len() > 5000 {
                        let mut end = 5000;
                        while end > 0 && !output.is_char_boundary(end) {
                            end -= 1;
                        }
                        format!(
                            "{}...[truncated, {} total chars]",
                            &output[..end],
                            output.len()
                        )
                    } else {
                        output
                    };
                    results.push(Content::ToolResult {
                        tool_use_id: tc.id.clone(),
                        output: serde_json::Value::String(truncated),
                        is_error: Some(is_error),
                    });
                }
                None => {
                    results.push(Content::ToolResult {
                        tool_use_id: tc.id.clone(),
                        output: serde_json::Value::String(format!("Tool '{}' not found", tc.name)),
                        is_error: Some(true),
                    });
                }
            }
        }

        // Add ALL tool results in a single user message (required by Anthropic API)
        self.add_tool_results(results).await;
    }

    /// Add all tool results in a single user message
    /// This is required by the API - tool results must follow their tool calls
    async fn add_tool_results(&self, results: Vec<Content>) {
        if results.is_empty() {
            return;
        }
        let mut messages = self.messages.write().await;
        messages.push(ModelMessage {
            role: Role::User,
            content: results,
        });
    }

    /// Classify the response to determine the dialogue result type
    ///
    /// Uses conservative classification: ambiguous responses default to
    /// NeedsEnhancement rather than silently approving.
    fn classify_response(&self, response: &str, turn: DialogueTurn) -> DialogueResult {
        let lower = response.to_lowercase();
        let word_count = response.split_whitespace().count();

        // Explicit approval patterns - must be clear and unambiguous
        let approval_patterns = [
            "proceed",
            "approved",
            "looks good",
            "no issues",
            "no concerns",
            "lgtm",
            "good to go",
            "correct approach",
            "appropriate",
        ];

        // Check for explicit approval (short responses with approval words)
        let has_approval = approval_patterns.iter().any(|p| lower.contains(p));

        // Concern patterns that indicate review needed
        let concern_patterns = [
            "enhance",
            "should be",
            "could be",
            "issue",
            "problem",
            "concern",
            "warning",
            "careful",
            "instead",
            "rather",
            "however",
            "but ",
            "although",
            "consider",
            "suggest",
            "recommend",
            "better to",
            "might want",
        ];

        let has_concern = concern_patterns.iter().any(|p| lower.contains(p));

        // Clear approval: has approval words, no concerns, short response
        if has_approval && !has_concern && word_count < 30 {
            return DialogueResult::Consensus {
                dialogue: vec![turn],
            };
        }

        // Clear concern: has concern patterns
        if has_concern {
            return DialogueResult::NeedsEnhancement {
                dialogue: vec![turn.clone()],
                critique: turn.content,
            };
        }

        // Ambiguous long responses should be reviewed (safety first)
        if word_count > 50 {
            debug!(
                word_count = word_count,
                "Long ambiguous response, treating as needing review"
            );
            return DialogueResult::NeedsEnhancement {
                dialogue: vec![turn.clone()],
                critique: turn.content,
            };
        }

        // Short ambiguous responses: default to consensus
        // (Little Claw was asked to be explicit, so short = likely ok)
        DialogueResult::Consensus {
            dialogue: vec![turn],
        }
    }

    /// Prune message history to prevent unbounded growth
    async fn prune_messages(&self) {
        let mut messages = self.messages.write().await;
        if messages.len() > 50 {
            let keep_from = messages.len() - 40;
            let first = messages.remove(0); // system prompt
            messages.drain(..keep_from - 1);
            messages.insert(0, first);
            debug!(
                new_len = messages.len(),
                "Pruned Little Claw message history"
            );
        }
    }

    /// Get the current message count
    pub async fn message_count(&self) -> usize {
        self.messages.read().await.len()
    }

    /// Get observation count
    pub async fn observation_count(&self) -> usize {
        self.observation_log.read().await.len()
    }

    /// Clear conversation history (for new session)
    pub async fn clear(&self) {
        self.messages.write().await.clear();
        *self.observation_cursor.write().await = 0;
    }
}

/// Check if a response is trivial (just an approval with no substance)
pub fn is_trivial_response(text: &str) -> bool {
    let lower = text.to_lowercase();
    let word_count = text.split_whitespace().count();

    // Very short responses are trivial
    if word_count < 10 {
        return true;
    }

    // Simple approval patterns
    let approval_patterns = [
        "proceed",
        "approved",
        "looks good",
        "lgtm",
        "no issues",
        "correct",
        "good to go",
    ];

    // If the response is mostly just an approval phrase
    for pattern in approval_patterns {
        if lower.contains(pattern) && word_count < 20 {
            return true;
        }
    }

    false
}

/// Extract generalizable insight patterns from review text
pub fn extract_insight_patterns(text: &str) -> Vec<String> {
    let mut insights = Vec::new();

    // Look for sentences that suggest generalizable patterns
    let insight_markers = [
        // Convention markers
        "this codebase uses",
        "the convention here is",
        "the pattern in this project",
        "consistently uses",
        // Pitfall markers
        "be careful",
        "avoid",
        "don't use",
        "shouldn't",
        "watch out for",
        // Architecture markers
        "the architecture",
        "this module",
        "the design pattern",
        // Best practice markers
        "best practice",
        "should always",
        "make sure to",
        "remember to",
        // Pattern/approach markers
        "the correct approach",
        "the existing pattern",
        "follows the pattern",
        "matches the existing",
        "consistent with",
        // Quality markers
        "should use",
        "prefer",
        "the idiomatic way",
        "rust convention",
        // Structure markers
        "this file handles",
        "this module is responsible",
        "the entry point",
    ];

    // Split into sentences and check each
    for sentence in text.split(['.', '!', '\n']) {
        let trimmed = sentence.trim();
        if trimmed.len() < 20 {
            continue;
        }

        let lower = trimmed.to_lowercase();
        for marker in insight_markers {
            if lower.contains(marker) {
                // Clean up the sentence
                let cleaned = trimmed
                    .trim_start_matches(|c: char| !c.is_alphanumeric())
                    .to_string();

                if cleaned.len() > 20 && !insights.contains(&cleaned) {
                    insights.push(cleaned);
                }
                break;
            }
        }
    }

    // Limit to avoid noise
    insights.truncate(3);
    insights
}
