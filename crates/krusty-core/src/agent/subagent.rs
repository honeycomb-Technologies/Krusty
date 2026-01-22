//! Sub-agent system for parallel task execution
//!
//! Enables spawning lightweight agents (e.g., Haiku) to explore the codebase.
//! Sub-agents have read-only access: glob, grep, read.
//! They cannot modify files or execute arbitrary commands.

use anyhow::Result;
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Semaphore};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::agent::build_context::SharedBuildContext;
use crate::agent::cache::SharedExploreCache;
use crate::agent::AgentCancellation;
use crate::ai::client::AiClient;
use crate::ai::providers::ProviderId;
use crate::ai::types::{AiTool, Content, ModelMessage, Role};
use crate::tools::implementations::{BashTool, EditTool, GlobTool, GrepTool, ReadTool, WriteTool};
use crate::tools::registry::{Tool, ToolContext, ToolResult};

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

/// Available models for sub-agents
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubAgentModel {
    /// Claude Haiku 4.5 - fast and cheap, ideal for exploration
    Haiku,
    /// Claude Sonnet 4.5 - balanced, good for analysis
    Sonnet,
    /// Claude Opus 4.5 - powerful, for builder agents
    Opus,
}

impl SubAgentModel {
    pub fn model_id(&self) -> &'static str {
        match self {
            // Use Claude 4 models
            SubAgentModel::Haiku => "claude-haiku-4-5-20251001",
            SubAgentModel::Sonnet => "claude-sonnet-4-5-20250929",
            SubAgentModel::Opus => "claude-opus-4-5-20251101",
        }
    }

    pub fn max_tokens(&self) -> usize {
        match self {
            SubAgentModel::Haiku => 4096,
            SubAgentModel::Sonnet => 8192,
            SubAgentModel::Opus => 16384,
        }
    }
}

/// Configuration for a sub-agent task
#[derive(Debug, Clone)]
pub struct SubAgentTask {
    pub id: String,
    /// Display name for the agent (e.g., "tui", "agent", "main")
    pub name: String,
    pub prompt: String,
    pub model: SubAgentModel,
    pub working_dir: PathBuf,
    /// Plan task ID this agent completes (for auto-marking)
    pub plan_task_id: Option<String>,
}

impl SubAgentTask {
    pub fn new(id: impl Into<String>, prompt: impl Into<String>) -> Self {
        let id = id.into();
        let name = id.clone(); // Default name is same as id
        Self {
            id,
            name,
            prompt: prompt.into(),
            model: SubAgentModel::Haiku,
            working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            plan_task_id: None,
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

    pub fn with_model(mut self, model: SubAgentModel) -> Self {
        self.model = model;
        self
    }

    pub fn with_plan_task_id(mut self, task_id: impl Into<String>) -> Self {
        self.plan_task_id = Some(task_id.into());
        self
    }

    fn system_prompt(&self) -> String {
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
}

/// Pool for managing concurrent sub-agent execution
pub struct SubAgentPool {
    client: Arc<AiClient>,
    cancellation: AgentCancellation,
    max_concurrency: usize,
    cache: Arc<SharedExploreCache>,
    /// Override model for non-Anthropic providers (uses user's selected model)
    override_model: Option<String>,
}

impl SubAgentPool {
    pub fn new(client: Arc<AiClient>, cancellation: AgentCancellation) -> Self {
        Self {
            client,
            cancellation,
            max_concurrency: 100, // No practical limit
            cache: Arc::new(SharedExploreCache::new()),
            override_model: None,
        }
    }

    pub fn with_concurrency(mut self, max: usize) -> Self {
        self.max_concurrency = max;
        self
    }

    /// Set an override model for non-Anthropic providers
    /// When set and provider isn't Anthropic, subagents use this model instead of SubAgentModel
    pub fn with_override_model(mut self, model: Option<String>) -> Self {
        self.override_model = model;
        self
    }

    /// Resolve which model to use for a task
    /// - Anthropic provider: use the task's specialized model (Haiku for explore, Opus for build)
    /// - Other providers: use the override model (user's selected model)
    fn resolve_model(&self, task: &SubAgentTask) -> String {
        if self.client.provider_id() == ProviderId::Anthropic {
            // Anthropic: use specialized subagent models
            task.model.model_id().to_string()
        } else {
            // Non-Anthropic: use the user's selected model (or fall back to task model)
            self.override_model
                .clone()
                .unwrap_or_else(|| task.model.model_id().to_string())
        }
    }

    /// Execute multiple sub-agent tasks concurrently
    pub async fn execute(&self, tasks: Vec<SubAgentTask>) -> Vec<SubAgentResult> {
        let semaphore = Arc::new(Semaphore::new(self.max_concurrency));
        let client = self.client.clone();
        let cancellation = self.cancellation.clone();
        let cache = self.cache.clone();
        let task_count = tasks.len();

        info!(
            count = task_count,
            concurrency = self.max_concurrency,
            "SubAgentPool: Spawning sub-agents"
        );

        let futures: Vec<_> = tasks
            .into_iter()
            .map(|task| {
                let sem = semaphore.clone();
                let client = client.clone();
                let cancel = cancellation.child_token();
                let cache = cache.clone();
                let task_id = task.id.clone();
                let resolved_model = self.resolve_model(&task);

                async move {
                    debug!(task_id = %task_id, "SubAgent: Acquiring semaphore permit");
                    let _permit = match sem.acquire().await {
                        Ok(p) => p,
                        Err(e) => {
                            warn!(task_id = %task_id, error = %e, "SubAgent: Failed to acquire semaphore");
                            return SubAgentResult {
                                task_id,
                                success: false,
                                output: String::new(),
                                files_examined: vec![],
                                duration_ms: 0,
                                turns_used: 0,
                                error: Some(format!("Semaphore error: {}", e)),
                            };
                        }
                    };
                    debug!(task_id = %task_id, "SubAgent: Got permit, checking cancellation");

                    if cancel.is_cancelled() {
                        info!(task_id = %task_id, "SubAgent: Cancelled before execution");
                        return SubAgentResult {
                            task_id,
                            success: false,
                            output: String::new(),
                            files_examined: vec![],
                            duration_ms: 0,
                            turns_used: 0,
                            error: Some("Cancelled".to_string()),
                        };
                    }

                    info!(task_id = %task_id, model = %resolved_model, "SubAgent: Starting execution");
                    let result = execute_subagent_with_tools(&client, task, &resolved_model, cancel, cache).await;
                    info!(task_id = %result.task_id, success = result.success, "SubAgent: Execution complete");
                    result
                }
            })
            .collect();

        info!("SubAgentPool: Waiting for {} futures", futures.len());
        let results: Vec<SubAgentResult> = join_all(futures).await;
        let stats = cache.stats();
        info!(
            "SubAgentPool: All futures complete, {} results | {}",
            results.len(),
            stats
        );
        results
    }

    /// Execute with real-time progress updates
    pub async fn execute_with_progress(
        &self,
        tasks: Vec<SubAgentTask>,
        progress_tx: mpsc::UnboundedSender<AgentProgress>,
    ) -> Vec<SubAgentResult> {
        let semaphore = Arc::new(Semaphore::new(self.max_concurrency));
        let client = self.client.clone();
        let cancellation = self.cancellation.clone();
        let cache = self.cache.clone();
        let task_count = tasks.len();

        info!(
            count = task_count,
            concurrency = self.max_concurrency,
            "SubAgentPool: Spawning sub-agents with progress"
        );

        let futures: Vec<_> = tasks
            .into_iter()
            .map(|task| {
                let sem = semaphore.clone();
                let client = client.clone();
                let cancel = cancellation.child_token();
                let cache = cache.clone();
                let task_id = task.id.clone();
                let progress_tx = progress_tx.clone();
                let resolved_model = self.resolve_model(&task);

                async move {
                    let _permit = match sem.acquire().await {
                        Ok(p) => p,
                        Err(e) => {
                            warn!(task_id = %task_id, error = %e, "SubAgent: Failed to acquire semaphore");
                            return SubAgentResult {
                                task_id,
                                success: false,
                                output: String::new(),
                                files_examined: vec![],
                                duration_ms: 0,
                                turns_used: 0,
                                error: Some(format!("Semaphore error: {}", e)),
                            };
                        }
                    };

                    if cancel.is_cancelled() {
                        return SubAgentResult {
                            task_id,
                            success: false,
                            output: String::new(),
                            files_examined: vec![],
                            duration_ms: 0,
                            turns_used: 0,
                            error: Some("Cancelled".to_string()),
                        };
                    }

                    execute_subagent_with_progress(&client, task, &resolved_model, cancel, cache, progress_tx).await
                }
            })
            .collect();

        let results: Vec<SubAgentResult> = join_all(futures).await;
        let stats = cache.stats();
        info!("SubAgentPool: Complete | {}", stats);
        results
    }

    /// Execute builder tasks with write access and shared context
    pub async fn execute_builders(
        &self,
        tasks: Vec<SubAgentTask>,
        context: Arc<SharedBuildContext>,
        progress_tx: mpsc::UnboundedSender<AgentProgress>,
    ) -> Vec<SubAgentResult> {
        let semaphore = Arc::new(Semaphore::new(self.max_concurrency));
        let client = self.client.clone();
        let cancellation = self.cancellation.clone();
        let task_count = tasks.len();

        info!(
            count = task_count,
            concurrency = self.max_concurrency,
            "SubAgentPool: Spawning builder agents"
        );

        let futures: Vec<_> = tasks
            .into_iter()
            .map(|task| {
                let sem = semaphore.clone();
                let client = client.clone();
                let cancel = cancellation.child_token();
                let context = context.clone();
                let task_id = task.id.clone();
                let progress_tx = progress_tx.clone();
                let resolved_model = self.resolve_model(&task);

                async move {
                    let _permit = match sem.acquire().await {
                        Ok(p) => p,
                        Err(e) => {
                            warn!(task_id = %task_id, error = %e, "Builder: Failed to acquire semaphore");
                            return SubAgentResult {
                                task_id,
                                success: false,
                                output: String::new(),
                                files_examined: vec![],
                                duration_ms: 0,
                                turns_used: 0,
                                error: Some(format!("Semaphore error: {}", e)),
                            };
                        }
                    };

                    if cancel.is_cancelled() {
                        return SubAgentResult {
                            task_id,
                            success: false,
                            output: String::new(),
                            files_examined: vec![],
                            duration_ms: 0,
                            turns_used: 0,
                            error: Some("Cancelled".to_string()),
                        };
                    }

                    execute_builder_with_progress(&client, task, &resolved_model, cancel, context, progress_tx).await
                }
            })
            .collect();

        let results: Vec<SubAgentResult> = join_all(futures).await;
        let stats = context.stats();
        info!("SubAgentPool: Builders complete | {}", stats);
        results
    }
}

/// Sub-agent tools - read-only access with shared cache
struct SubAgentTools {
    glob: GlobTool,
    grep: GrepTool,
    read: ReadTool,
    cache: Arc<SharedExploreCache>,
}

impl SubAgentTools {
    fn new(cache: Arc<SharedExploreCache>) -> Self {
        Self {
            glob: GlobTool,
            grep: GrepTool,
            read: ReadTool,
            cache,
        }
    }

    fn get_ai_tools(&self) -> Vec<AiTool> {
        vec![
            AiTool {
                name: "glob".to_string(),
                description: self.glob.description().to_string(),
                input_schema: self.glob.parameters_schema(),
            },
            AiTool {
                name: "grep".to_string(),
                description: self.grep.description().to_string(),
                input_schema: self.grep.parameters_schema(),
            },
            AiTool {
                name: "read".to_string(),
                description: self.read.description().to_string(),
                input_schema: self.read.parameters_schema(),
            },
        ]
    }

    async fn execute(&self, name: &str, params: Value, ctx: &ToolContext) -> Option<ToolResult> {
        match name {
            "glob" => {
                // Check cache for glob results
                let pattern = params
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let base_dir = ctx.working_dir.clone();

                if let Some(cached_paths) = self.cache.get_glob(&pattern, &base_dir) {
                    // Return cached result formatted as the tool would
                    let output = cached_paths
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join("\n");
                    return Some(ToolResult {
                        output: if output.is_empty() {
                            "No matches found".to_string()
                        } else {
                            output
                        },
                        is_error: false,
                    });
                }

                // Execute and cache
                let result = self.glob.execute(params, ctx).await;
                if !result.is_error {
                    // Parse paths from output and cache
                    let paths: Vec<PathBuf> = result
                        .output
                        .lines()
                        .filter(|l| !l.is_empty() && *l != "No matches found")
                        .map(PathBuf::from)
                        .collect();
                    self.cache.put_glob(pattern, base_dir, paths);
                }
                Some(result)
            }
            "grep" => {
                // Grep caching is trickier due to many parameters
                // For now, just execute without caching (grep results vary by flags)
                Some(self.grep.execute(params, ctx).await)
            }
            "read" => {
                // Check cache for file content
                let file_path = params
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .map(PathBuf::from);

                if let Some(path) = file_path {
                    // Only cache full file reads (no offset/limit)
                    let has_offset = params.get("offset").is_some();
                    let has_limit = params.get("limit").is_some();

                    if !has_offset && !has_limit {
                        if let Some(cached) = self.cache.get_file(&path) {
                            // Format like the read tool does (with line numbers)
                            let output = cached
                                .content
                                .lines()
                                .enumerate()
                                .map(|(i, line)| format!("{:>6}→{}", i + 1, line))
                                .collect::<Vec<_>>()
                                .join("\n");
                            return Some(ToolResult {
                                output,
                                is_error: false,
                            });
                        }
                    }

                    // Execute and cache (only full reads)
                    let result = self.read.execute(params, ctx).await;
                    if !result.is_error && !has_offset && !has_limit {
                        // Extract raw content (strip line numbers)
                        let raw_content: String = result
                            .output
                            .lines()
                            .map(|line| {
                                // Line format: "    123→content" - find the → and take after it
                                if let Some(pos) = line.find('→') {
                                    &line[pos + '→'.len_utf8()..]
                                } else {
                                    line
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        self.cache.put_file(path, raw_content);
                    }
                    Some(result)
                } else {
                    Some(self.read.execute(params, ctx).await)
                }
            }
            _ => None,
        }
    }
}

/// Builder agent tools - read/write access with shared build context
pub struct BuilderTools {
    glob: GlobTool,
    grep: GrepTool,
    read: ReadTool,
    write: WriteTool,
    edit: EditTool,
    bash: BashTool,
    context: Arc<SharedBuildContext>,
    builder_id: String,
}

impl BuilderTools {
    pub fn new(context: Arc<SharedBuildContext>, builder_id: String) -> Self {
        Self {
            glob: GlobTool,
            grep: GrepTool,
            read: ReadTool,
            write: WriteTool,
            edit: EditTool,
            bash: BashTool,
            context,
            builder_id,
        }
    }

    /// Try to acquire a file lock with exponential backoff (fast for brief locks)
    async fn acquire_lock_with_retry(&self, path: &std::path::Path) -> Result<(), String> {
        let path_buf = path.to_path_buf();
        // Fast exponential backoff: 50ms, 100ms, 200ms, 400ms, 800ms, 1s, 1s, 1s, 1s, 1s = ~6s total
        let delays_ms = [50, 100, 200, 400, 800, 1000, 1000, 1000, 1000, 1000];

        for (attempt, delay) in delays_ms.iter().enumerate() {
            match self.context.acquire_lock(
                path_buf.clone(),
                self.builder_id.clone(),
                "write/edit".to_string(),
            ) {
                Ok(()) => return Ok(()),
                Err(holder) => {
                    if attempt < delays_ms.len() - 1 {
                        tracing::debug!(
                            builder = %self.builder_id,
                            path = %path.display(),
                            holder = %holder,
                            attempt = attempt,
                            "File locked, backoff {}ms",
                            delay
                        );
                        tokio::time::sleep(tokio::time::Duration::from_millis(*delay)).await;
                    } else {
                        return Err(format!(
                            "File {} locked by {} (tried {}x)",
                            path.display(),
                            holder,
                            delays_ms.len()
                        ));
                    }
                }
            }
        }
        Err("Lock acquisition failed".to_string())
    }

    pub fn get_ai_tools(&self) -> Vec<AiTool> {
        vec![
            AiTool {
                name: "glob".to_string(),
                description: self.glob.description().to_string(),
                input_schema: self.glob.parameters_schema(),
            },
            AiTool {
                name: "grep".to_string(),
                description: self.grep.description().to_string(),
                input_schema: self.grep.parameters_schema(),
            },
            AiTool {
                name: "read".to_string(),
                description: self.read.description().to_string(),
                input_schema: self.read.parameters_schema(),
            },
            AiTool {
                name: "write".to_string(),
                description: self.write.description().to_string(),
                input_schema: self.write.parameters_schema(),
            },
            AiTool {
                name: "edit".to_string(),
                description: self.edit.description().to_string(),
                input_schema: self.edit.parameters_schema(),
            },
            AiTool {
                name: "bash".to_string(),
                description: self.bash.description().to_string(),
                input_schema: self.bash.parameters_schema(),
            },
        ]
    }

    pub async fn execute(
        &self,
        name: &str,
        params: Value,
        ctx: &ToolContext,
    ) -> Option<ToolResult> {
        match name {
            "glob" => Some(self.glob.execute(params, ctx).await),
            "grep" => Some(self.grep.execute(params, ctx).await),
            "read" => Some(self.read.execute(params, ctx).await),
            "write" => {
                // Get file path and acquire lock before writing
                let path = match params.get("file_path").and_then(|v| v.as_str()) {
                    Some(p) => PathBuf::from(p),
                    None => {
                        return Some(ToolResult {
                            output: "Missing file_path parameter".to_string(),
                            is_error: true,
                        })
                    }
                };

                // Acquire lock with retry (waits for other builders)
                if let Err(e) = self.acquire_lock_with_retry(&path).await {
                    return Some(ToolResult {
                        output: format!("Cannot write: {}", e),
                        is_error: true,
                    });
                }

                let result = self.write.execute(params.clone(), ctx).await;

                // Track line changes for the build context
                if !result.is_error {
                    if let Some(content) = params.get("content").and_then(|v| v.as_str()) {
                        let lines_added = content.lines().count();
                        self.context.record_line_changes(lines_added, 0);
                    }
                    self.context
                        .record_modification(path.clone(), self.builder_id.clone());
                }

                // Release lock after write
                self.context.release_lock(&path, &self.builder_id);
                Some(result)
            }
            "edit" => {
                // Get file path and acquire lock before editing
                let path = match params.get("file_path").and_then(|v| v.as_str()) {
                    Some(p) => PathBuf::from(p),
                    None => {
                        return Some(ToolResult {
                            output: "Missing file_path parameter".to_string(),
                            is_error: true,
                        })
                    }
                };

                // Acquire lock with retry (waits for other builders)
                if let Err(e) = self.acquire_lock_with_retry(&path).await {
                    return Some(ToolResult {
                        output: format!("Cannot edit: {}", e),
                        is_error: true,
                    });
                }

                let result = self.edit.execute(params.clone(), ctx).await;

                // Track line changes for edits
                if !result.is_error {
                    let old_lines = params
                        .get("old_string")
                        .and_then(|v| v.as_str())
                        .map(|s| s.lines().count())
                        .unwrap_or(0);
                    let new_lines = params
                        .get("new_string")
                        .and_then(|v| v.as_str())
                        .map(|s| s.lines().count())
                        .unwrap_or(0);
                    if new_lines > old_lines {
                        self.context.record_line_changes(new_lines - old_lines, 0);
                    } else {
                        self.context.record_line_changes(0, old_lines - new_lines);
                    }
                    self.context
                        .record_modification(path.clone(), self.builder_id.clone());
                }

                // Release lock after edit
                self.context.release_lock(&path, &self.builder_id);
                Some(result)
            }
            "bash" => Some(self.bash.execute(params, ctx).await),
            _ => None,
        }
    }
}

/// Execute a sub-agent with agentic tool loop
async fn execute_subagent_with_tools(
    client: &AiClient,
    task: SubAgentTask,
    model: &str,
    cancellation: CancellationToken,
    cache: Arc<SharedExploreCache>,
) -> SubAgentResult {
    let start = Instant::now();
    let task_id = task.id.clone();

    debug!(task_id = %task_id, model = %model, "Starting sub-agent");

    let tools = SubAgentTools::new(cache);
    let ai_tools = tools.get_ai_tools();
    let system_prompt = task.system_prompt();

    let ctx = ToolContext {
        working_dir: task.working_dir.clone(),
        sandbox_root: None,
        user_id: None,
        lsp_manager: None,
        process_registry: None,
        skills_manager: None,
        mcp_manager: None,
        timeout: Some(Duration::from_secs(30)),
        output_tx: None,
        tool_use_id: None,
        plan_mode: false,
        explore_progress_tx: None,
        build_progress_tx: None,
        missing_lsp_tx: None,
        current_model: None,
    };

    // Build initial message
    let mut messages: Vec<ModelMessage> = vec![ModelMessage {
        role: Role::User,
        content: vec![Content::Text {
            text: task.prompt.clone(),
        }],
    }];

    let mut files_examined: Vec<String> = vec![];
    let mut turns = 0;
    let mut final_output = String::new();

    // Agentic loop - runs until agent naturally completes or is cancelled
    loop {
        if cancellation.is_cancelled() {
            info!(task_id = %task_id, "SubAgent cancelled");
            return SubAgentResult {
                task_id,
                success: false,
                output: String::new(),
                files_examined,
                duration_ms: start.elapsed().as_millis() as u64,
                turns_used: turns,
                error: Some("Cancelled".to_string()),
            };
        }

        turns += 1;
        info!(task_id = %task_id, turn = turns, "SubAgent making API call");

        // Make API call
        let response = match call_subagent_api(
            client,
            model,
            &system_prompt,
            &messages,
            &ai_tools,
            task.model.max_tokens(),
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                return SubAgentResult {
                    task_id,
                    success: false,
                    output: String::new(),
                    files_examined,
                    duration_ms: start.elapsed().as_millis() as u64,
                    turns_used: turns,
                    error: Some(e.to_string()),
                };
            }
        };

        // Parse response
        let (text_parts, tool_calls, stop_reason) = parse_response(&response);
        info!(
            task_id = %task_id,
            turn = turns,
            text_parts = text_parts.len(),
            tool_calls = tool_calls.len(),
            stop_reason = %stop_reason,
            "SubAgent parsed API response"
        );

        // Collect any text output
        if !text_parts.is_empty() {
            final_output = text_parts.join("\n");
        }

        // If no tool calls or stop_reason is end_turn, we're done
        if tool_calls.is_empty() || stop_reason == "end_turn" {
            info!(task_id = %task_id, turns = turns, output_len = final_output.len(), "Sub-agent completed successfully");
            return SubAgentResult {
                task_id,
                success: true,
                output: final_output,
                files_examined,
                duration_ms: start.elapsed().as_millis() as u64,
                turns_used: turns,
                error: None,
            };
        }

        // Add assistant message with tool calls
        let mut assistant_content: Vec<Content> = text_parts
            .iter()
            .map(|t| Content::Text { text: t.clone() })
            .collect();

        for tc in &tool_calls {
            assistant_content.push(Content::ToolUse {
                id: tc.id.clone(),
                name: tc.name.clone(),
                input: tc.input.clone(),
            });
        }

        messages.push(ModelMessage {
            role: Role::Assistant,
            content: assistant_content,
        });

        // Execute tools and collect results
        info!(task_id = %task_id, tool_count = tool_calls.len(), "SubAgent executing tools");
        let mut tool_results: Vec<Content> = vec![];

        for tc in &tool_calls {
            debug!(task_id = %task_id, tool = %tc.name, "SubAgent executing tool");

            // Track files examined
            if tc.name == "read" {
                if let Some(path) = tc.input.get("file_path").and_then(|v| v.as_str()) {
                    files_examined.push(path.to_string());
                }
            }

            let result = tools.execute(&tc.name, tc.input.clone(), &ctx).await;

            let (output, is_error) = match result {
                Some(r) => (r.output, r.is_error),
                None => (format!("Unknown tool: {}", tc.name), true),
            };

            tool_results.push(Content::ToolResult {
                tool_use_id: tc.id.clone(),
                output: Value::String(output.clone()),
                is_error: Some(is_error),
            });

            debug!(
                task_id = %task_id,
                tool = %tc.name,
                output_len = output.len(),
                is_error = is_error,
                "SubAgent tool completed"
            );
        }

        info!(task_id = %task_id, "SubAgent tools executed, continuing loop");

        // Add user message with tool results
        messages.push(ModelMessage {
            role: Role::User,
            content: tool_results,
        });
    }
}

/// Execute a sub-agent with progress reporting
async fn execute_subagent_with_progress(
    client: &AiClient,
    task: SubAgentTask,
    model: &str,
    cancellation: CancellationToken,
    cache: Arc<SharedExploreCache>,
    progress_tx: mpsc::UnboundedSender<AgentProgress>,
) -> SubAgentResult {
    let start = Instant::now();
    let task_id = task.id.clone();
    let task_name = task.name.clone();

    let tools = SubAgentTools::new(cache);
    let ai_tools = tools.get_ai_tools();
    let system_prompt = task.system_prompt();

    let ctx = ToolContext {
        working_dir: task.working_dir.clone(),
        sandbox_root: None,
        user_id: None,
        lsp_manager: None,
        process_registry: None,
        skills_manager: None,
        mcp_manager: None,
        timeout: Some(Duration::from_secs(30)),
        output_tx: None,
        tool_use_id: None,
        plan_mode: false,
        explore_progress_tx: None,
        build_progress_tx: None,
        missing_lsp_tx: None,
        current_model: None,
    };

    let mut messages: Vec<ModelMessage> = vec![ModelMessage {
        role: Role::User,
        content: vec![Content::Text {
            text: task.prompt.clone(),
        }],
    }];

    let mut files_examined: Vec<String> = vec![];
    let mut turns = 0;
    let mut total_tool_calls = 0;
    let mut estimated_tokens: usize = 0;
    let mut final_output = String::new();
    let mut last_action = "starting...".to_string();

    // Send initial progress
    let _ = progress_tx.send(AgentProgress {
        task_id: task_id.clone(),
        name: task_name.clone(),
        status: AgentProgressStatus::Running,
        tool_count: 0,
        tokens: 0,
        current_action: Some(last_action.clone()),
        ..Default::default()
    });

    loop {
        if cancellation.is_cancelled() {
            let _ = progress_tx.send(AgentProgress {
                task_id: task_id.clone(),
                name: task_name.clone(),
                status: AgentProgressStatus::Failed,
                tool_count: total_tool_calls,
                tokens: estimated_tokens,
                current_action: Some("cancelled".to_string()),
                ..Default::default()
            });
            return SubAgentResult {
                task_id,
                success: false,
                output: String::new(),
                files_examined,
                duration_ms: start.elapsed().as_millis() as u64,
                turns_used: turns,
                error: Some("Cancelled".to_string()),
            };
        }

        turns += 1;

        // Send progress: making API call (show last action or "thinking...")
        let thinking_action = if total_tool_calls > 0 {
            format!("{}...", last_action)
        } else {
            "thinking...".to_string()
        };
        let _ = progress_tx.send(AgentProgress {
            task_id: task_id.clone(),
            name: task_name.clone(),
            status: AgentProgressStatus::Running,
            tool_count: total_tool_calls,
            tokens: estimated_tokens,
            current_action: Some(thinking_action),
            ..Default::default()
        });

        let response = match call_subagent_api(
            client,
            model,
            &system_prompt,
            &messages,
            &ai_tools,
            task.model.max_tokens(),
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                let _ = progress_tx.send(AgentProgress {
                    task_id: task_id.clone(),
                    name: task_name.clone(),
                    status: AgentProgressStatus::Failed,
                    tool_count: total_tool_calls,
                    tokens: estimated_tokens,
                    current_action: Some("error".to_string()),
                    ..Default::default()
                });
                return SubAgentResult {
                    task_id,
                    success: false,
                    output: String::new(),
                    files_examined,
                    duration_ms: start.elapsed().as_millis() as u64,
                    turns_used: turns,
                    error: Some(e.to_string()),
                };
            }
        };

        // Estimate tokens from response
        if let Some(usage) = response.get("usage") {
            if let Some(input) = usage.get("input_tokens").and_then(|v| v.as_u64()) {
                estimated_tokens += input as usize;
            }
            if let Some(output) = usage.get("output_tokens").and_then(|v| v.as_u64()) {
                estimated_tokens += output as usize;
            }
        }

        let (text_parts, tool_calls, stop_reason) = parse_response(&response);

        if !text_parts.is_empty() {
            final_output = text_parts.join("\n");
        }

        if tool_calls.is_empty() || stop_reason == "end_turn" {
            // Send completion progress
            let _ = progress_tx.send(AgentProgress {
                task_id: task_id.clone(),
                name: task_name.clone(),
                status: AgentProgressStatus::Complete,
                tool_count: total_tool_calls,
                tokens: estimated_tokens,
                current_action: Some("complete".to_string()),
                ..Default::default()
            });
            return SubAgentResult {
                task_id,
                success: true,
                output: final_output,
                files_examined,
                duration_ms: start.elapsed().as_millis() as u64,
                turns_used: turns,
                error: None,
            };
        }

        // Add assistant message
        let mut assistant_content: Vec<Content> = text_parts
            .iter()
            .map(|t| Content::Text { text: t.clone() })
            .collect();

        for tc in &tool_calls {
            assistant_content.push(Content::ToolUse {
                id: tc.id.clone(),
                name: tc.name.clone(),
                input: tc.input.clone(),
            });
        }

        messages.push(ModelMessage {
            role: Role::Assistant,
            content: assistant_content,
        });

        // Execute tools
        let mut tool_results: Vec<Content> = vec![];

        for tc in &tool_calls {
            total_tool_calls += 1;

            // Build action description
            last_action = match tc.name.as_str() {
                "read" => {
                    let path = tc
                        .input
                        .get("file_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    let short_path = path.rsplit('/').next().unwrap_or(path);
                    files_examined.push(path.to_string());
                    format!("read {}", short_path)
                }
                "glob" => {
                    let pattern = tc
                        .input
                        .get("pattern")
                        .and_then(|v| v.as_str())
                        .unwrap_or("*");
                    format!("glob {}", pattern)
                }
                "grep" => {
                    let pattern = tc
                        .input
                        .get("pattern")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    let short = if pattern.len() > 12 {
                        &pattern[..12]
                    } else {
                        pattern
                    };
                    format!("grep {}", short)
                }
                _ => tc.name.clone(),
            };

            // Send progress with current action
            let _ = progress_tx.send(AgentProgress {
                task_id: task_id.clone(),
                name: task_name.clone(),
                status: AgentProgressStatus::Running,
                tool_count: total_tool_calls,
                tokens: estimated_tokens,
                current_action: Some(last_action.clone()),
                ..Default::default()
            });

            let result = tools.execute(&tc.name, tc.input.clone(), &ctx).await;

            let (output, is_error) = match result {
                Some(r) => (r.output, r.is_error),
                None => (format!("Unknown tool: {}", tc.name), true),
            };

            tool_results.push(Content::ToolResult {
                tool_use_id: tc.id.clone(),
                output: Value::String(output),
                is_error: Some(is_error),
            });
        }

        messages.push(ModelMessage {
            role: Role::User,
            content: tool_results,
        });
    }
}

#[derive(Debug)]
struct ToolCall {
    id: String,
    name: String,
    input: Value,
}

/// Make a non-streaming API call for sub-agent with timeout
async fn call_subagent_api(
    client: &AiClient,
    model: &str,
    system: &str,
    messages: &[ModelMessage],
    tools: &[AiTool],
    max_tokens: usize,
) -> Result<Value> {
    info!(
        model = model,
        msg_count = messages.len(),
        "SubAgent API call starting"
    );
    let start = Instant::now();
    // Build messages JSON
    let messages_json: Vec<Value> = messages
        .iter()
        .map(|m| {
            let role = match m.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => "user",
                Role::Tool => "user",
            };

            let content: Vec<Value> = m
                .content
                .iter()
                .map(|c| match c {
                    Content::Text { text } => json!({"type": "text", "text": text}),
                    Content::ToolUse { id, name, input } => {
                        json!({"type": "tool_use", "id": id, "name": name, "input": input})
                    }
                    Content::ToolResult {
                        tool_use_id,
                        output,
                        is_error,
                    } => json!({
                        "type": "tool_result",
                        "tool_use_id": tool_use_id,
                        "content": output,
                        "is_error": is_error.unwrap_or(false)
                    }),
                    _ => json!({"type": "text", "text": "[unsupported content]"}),
                })
                .collect();

            json!({"role": role, "content": content})
        })
        .collect();

    // Build tools JSON
    let tools_json: Vec<Value> = tools
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema
            })
        })
        .collect();

    // Use standard HTTP timeout (configured in client) - no additional timeout wrapper
    // The HTTP client already has a 10-minute timeout for streaming responses
    let result = client
        .call_with_tools(model, system, messages_json, tools_json, max_tokens)
        .await;
    let elapsed = start.elapsed();
    info!(
        elapsed_ms = elapsed.as_millis() as u64,
        success = result.is_ok(),
        "SubAgent API call completed"
    );
    result
}

/// Parse API response to extract text, tool calls, and stop reason
fn parse_response(response: &Value) -> (Vec<String>, Vec<ToolCall>, String) {
    let mut texts = vec![];
    let mut tool_calls = vec![];

    let stop_reason = response
        .get("stop_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if let Some(content) = response.get("content").and_then(|c| c.as_array()) {
        for block in content {
            let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");

            match block_type {
                "text" => {
                    if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                        texts.push(text.to_string());
                    }
                }
                "tool_use" => {
                    let id = block
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = block
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let input = block.get("input").cloned().unwrap_or(json!({}));

                    tool_calls.push(ToolCall { id, name, input });
                }
                _ => {}
            }
        }
    }

    (texts, tool_calls, stop_reason)
}

/// Generate builder system prompt with context injection
fn builder_system_prompt(working_dir: &std::path::Path, context: &SharedBuildContext) -> String {
    let context_injection = context.generate_context_injection();
    format!(
        r#"You are a builder agent. Your task is to implement code changes.

## Working Directory
{}

## Available Tools
1. **glob** - Find files by pattern (e.g., `**/*.rs`)
2. **grep** - Search file contents with regex
3. **read** - Read file contents (ALWAYS read before editing)
4. **write** - Write new files
5. **edit** - Edit existing files (requires reading first)
6. **bash** - Run shell commands

## Rules
1. ALWAYS read files before editing - other builders may have modified them
2. Create your OWN files for new components when possible
3. File locks are automatic - brief wait if another builder is writing
4. Follow [CONVENTIONS] if provided below
5. Be precise with edits - match exact strings

## Process
1. Use glob/grep to find relevant files
2. Read files you need to modify
3. Make your changes with write/edit
4. Summarize what you created/modified

{}

Build your component, then summarize what you created with file paths."#,
        working_dir.display(),
        context_injection
    )
}

/// Execute a builder agent with progress reporting
async fn execute_builder_with_progress(
    client: &AiClient,
    task: SubAgentTask,
    model: &str,
    cancellation: CancellationToken,
    context: Arc<SharedBuildContext>,
    progress_tx: mpsc::UnboundedSender<AgentProgress>,
) -> SubAgentResult {
    let start = Instant::now();
    let task_id = task.id.clone();
    let task_name = task.name.clone();
    let plan_task_id = task.plan_task_id.clone();

    let tools = BuilderTools::new(context.clone(), task_id.clone());
    let ai_tools = tools.get_ai_tools();

    let ctx = ToolContext {
        working_dir: task.working_dir.clone(),
        sandbox_root: None,
        user_id: None,
        lsp_manager: None,
        process_registry: None,
        skills_manager: None,
        mcp_manager: None,
        timeout: Some(Duration::from_secs(120)), // Builders get more time
        output_tx: None,
        tool_use_id: None,
        plan_mode: false,
        explore_progress_tx: None,
        build_progress_tx: None,
        missing_lsp_tx: None,
        current_model: None,
    };

    let mut messages: Vec<ModelMessage> = vec![ModelMessage {
        role: Role::User,
        content: vec![Content::Text {
            text: task.prompt.clone(),
        }],
    }];

    let mut files_examined: Vec<String> = vec![];
    let mut turns = 0;
    let mut total_tool_calls = 0;
    let mut estimated_tokens: usize = 0;
    let mut final_output = String::new();
    let mut last_action = "starting...".to_string();

    // Send initial progress
    let _ = progress_tx.send(AgentProgress {
        task_id: task_id.clone(),
        name: task_name.clone(),
        status: AgentProgressStatus::Running,
        tool_count: 0,
        tokens: 0,
        current_action: Some(last_action.clone()),
        ..Default::default()
    });

    loop {
        if cancellation.is_cancelled() {
            let (lines_added, lines_removed) = context.get_line_diff();
            let _ = progress_tx.send(AgentProgress {
                task_id: task_id.clone(),
                name: task_name.clone(),
                status: AgentProgressStatus::Failed,
                tool_count: total_tool_calls,
                tokens: estimated_tokens,
                current_action: Some("cancelled".to_string()),
                lines_added,
                lines_removed,
                completed_plan_task: None,
            });
            // Release any locks this builder held
            context.release_all_locks(&task_id);
            return SubAgentResult {
                task_id,
                success: false,
                output: String::new(),
                files_examined,
                duration_ms: start.elapsed().as_millis() as u64,
                turns_used: turns,
                error: Some("Cancelled".to_string()),
            };
        }

        turns += 1;

        // Re-inject context each turn so builders see updates from other builders
        let dynamic_system = builder_system_prompt(&task.working_dir, &context);

        let thinking_action = if total_tool_calls > 0 {
            format!("{}...", last_action)
        } else {
            "thinking...".to_string()
        };
        let (lines_added, lines_removed) = context.get_line_diff();
        let _ = progress_tx.send(AgentProgress {
            task_id: task_id.clone(),
            name: task_name.clone(),
            status: AgentProgressStatus::Running,
            tool_count: total_tool_calls,
            tokens: estimated_tokens,
            current_action: Some(thinking_action),
            lines_added,
            lines_removed,
            completed_plan_task: None,
        });

        let response = match call_subagent_api(
            client,
            model,
            &dynamic_system,
            &messages,
            &ai_tools,
            task.model.max_tokens(),
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                let (lines_added, lines_removed) = context.get_line_diff();
                let _ = progress_tx.send(AgentProgress {
                    task_id: task_id.clone(),
                    name: task_name.clone(),
                    status: AgentProgressStatus::Failed,
                    tool_count: total_tool_calls,
                    tokens: estimated_tokens,
                    current_action: Some("error".to_string()),
                    lines_added,
                    lines_removed,
                    completed_plan_task: None,
                });
                context.release_all_locks(&task_id);
                return SubAgentResult {
                    task_id,
                    success: false,
                    output: String::new(),
                    files_examined,
                    duration_ms: start.elapsed().as_millis() as u64,
                    turns_used: turns,
                    error: Some(e.to_string()),
                };
            }
        };

        // Estimate tokens from response
        if let Some(usage) = response.get("usage") {
            if let Some(input) = usage.get("input_tokens").and_then(|v| v.as_u64()) {
                estimated_tokens += input as usize;
            }
            if let Some(output) = usage.get("output_tokens").and_then(|v| v.as_u64()) {
                estimated_tokens += output as usize;
            }
        }

        let (text_parts, tool_calls, stop_reason) = parse_response(&response);

        if !text_parts.is_empty() {
            final_output = text_parts.join("\n");
        }

        if tool_calls.is_empty() || stop_reason == "end_turn" {
            let (lines_added, lines_removed) = context.get_line_diff();
            let _ = progress_tx.send(AgentProgress {
                task_id: task_id.clone(),
                name: task_name.clone(),
                status: AgentProgressStatus::Complete,
                tool_count: total_tool_calls,
                tokens: estimated_tokens,
                current_action: Some("complete".to_string()),
                lines_added,
                lines_removed,
                completed_plan_task: plan_task_id.clone(),
            });
            context.release_all_locks(&task_id);
            return SubAgentResult {
                task_id,
                success: true,
                output: final_output,
                files_examined,
                duration_ms: start.elapsed().as_millis() as u64,
                turns_used: turns,
                error: None,
            };
        }

        // Add assistant message
        let mut assistant_content: Vec<Content> = text_parts
            .iter()
            .map(|t| Content::Text { text: t.clone() })
            .collect();

        for tc in &tool_calls {
            assistant_content.push(Content::ToolUse {
                id: tc.id.clone(),
                name: tc.name.clone(),
                input: tc.input.clone(),
            });
        }

        messages.push(ModelMessage {
            role: Role::Assistant,
            content: assistant_content,
        });

        // Execute tools
        let mut tool_results: Vec<Content> = vec![];

        for tc in &tool_calls {
            total_tool_calls += 1;

            // Build action description
            last_action = match tc.name.as_str() {
                "read" => {
                    let path = tc
                        .input
                        .get("file_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    let short_path = path.rsplit('/').next().unwrap_or(path);
                    files_examined.push(path.to_string());
                    format!("read {}", short_path)
                }
                "write" => {
                    let path = tc
                        .input
                        .get("file_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    let short_path = path.rsplit('/').next().unwrap_or(path);
                    format!("write {}", short_path)
                }
                "edit" => {
                    let path = tc
                        .input
                        .get("file_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    let short_path = path.rsplit('/').next().unwrap_or(path);
                    format!("edit {}", short_path)
                }
                "glob" => {
                    let pattern = tc
                        .input
                        .get("pattern")
                        .and_then(|v| v.as_str())
                        .unwrap_or("*");
                    format!("glob {}", pattern)
                }
                "grep" => {
                    let pattern = tc
                        .input
                        .get("pattern")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    let short = if pattern.len() > 12 {
                        &pattern[..12]
                    } else {
                        pattern
                    };
                    format!("grep {}", short)
                }
                "bash" => {
                    let cmd = tc
                        .input
                        .get("command")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    let short = if cmd.len() > 15 { &cmd[..15] } else { cmd };
                    format!("bash {}", short)
                }
                _ => tc.name.clone(),
            };

            let (lines_added, lines_removed) = context.get_line_diff();
            let _ = progress_tx.send(AgentProgress {
                task_id: task_id.clone(),
                name: task_name.clone(),
                status: AgentProgressStatus::Running,
                tool_count: total_tool_calls,
                tokens: estimated_tokens,
                current_action: Some(last_action.clone()),
                lines_added,
                lines_removed,
                completed_plan_task: None,
            });

            let result = tools.execute(&tc.name, tc.input.clone(), &ctx).await;

            let (output, is_error) = match result {
                Some(r) => (r.output, r.is_error),
                None => (format!("Unknown tool: {}", tc.name), true),
            };

            tool_results.push(Content::ToolResult {
                tool_use_id: tc.id.clone(),
                output: Value::String(output),
                is_error: Some(is_error),
            });
        }

        messages.push(ModelMessage {
            role: Role::User,
            content: tool_results,
        });
    }
}
