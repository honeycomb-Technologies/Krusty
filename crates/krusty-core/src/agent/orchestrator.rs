//! Agentic orchestrator — the single canonical agentic loop.
//!
//! `AgenticOrchestrator` encapsulates the complete AI agent loop:
//! streaming, tool execution, context injection, plan management,
//! failure detection, and title generation.
//!
//! Both the TUI and HTTP server are thin presentation layers that:
//! - Create an orchestrator from their own state
//! - Call `run()` to get an event stream and input channel
//! - Map `LoopEvent` to their display format
//! - Send `LoopInput` for user interactions
//!
//! ```text
//!  ┌─────────────┐        LoopEvent         ┌─────────────┐
//!  │ Orchestrator │ ─────────────────────►   │  Consumer   │
//!  │   (core)     │                          │ (TUI/Server)│
//!  │              │ ◄─────────────────────   │             │
//!  └─────────────┘        LoopInput          └─────────────┘
//! ```

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{mpsc, RwLock};

use crate::ai::client::{AiClient, CallOptions};
use crate::ai::title::generate_title as ai_generate_title;
use crate::ai::types::{Content, ModelMessage, Role};
use crate::plan::PlanManager;
use crate::process::ProcessRegistry;
use crate::skills::SkillsManager;
use crate::storage::{Database, SessionManager, WorkMode};
use crate::tools::registry::{PermissionMode, ToolRegistry};

use super::context;
use super::executor;
use super::failure;
use super::loop_events::{LoopEvent, LoopInput, PlanTaskInfo};
use super::plan_handler;
use super::stream::{self, ThinkingBlock};

const MAX_ITERATIONS: usize = 50;
const EXPLORATION_BUDGET_SOFT: usize = 15;
const EXPLORATION_BUDGET_HARD: usize = 30;

/// Configuration for an orchestrator run.
pub struct OrchestratorConfig {
    pub session_id: String,
    pub working_dir: PathBuf,
    pub permission_mode: PermissionMode,
    pub max_iterations: usize,
    pub user_id: Option<String>,
    pub initial_work_mode: WorkMode,
    /// Whether to generate a title on first AI response.
    /// Set to true for new sessions, false for resumed conversations.
    pub generate_title: bool,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            working_dir: PathBuf::new(),
            permission_mode: PermissionMode::default(),
            max_iterations: MAX_ITERATIONS,
            user_id: None,
            initial_work_mode: WorkMode::default(),
            generate_title: false,
        }
    }
}

/// Shared services the orchestrator needs.
pub struct OrchestratorServices {
    pub ai_client: Arc<AiClient>,
    pub tool_registry: Arc<ToolRegistry>,
    pub process_registry: Arc<ProcessRegistry>,
    pub db_path: PathBuf,
    pub skills_manager: Arc<RwLock<SkillsManager>>,
}

/// The agentic orchestrator — runs the complete AI agent loop.
pub struct AgenticOrchestrator {
    services: OrchestratorServices,
    config: OrchestratorConfig,
}

impl AgenticOrchestrator {
    pub fn new(services: OrchestratorServices, config: OrchestratorConfig) -> Self {
        Self { services, config }
    }

    /// Start the agentic loop.
    ///
    /// Returns `(event_receiver, input_sender)`. The loop runs as a spawned
    /// tokio task. It emits `LoopEvent`s for every state change. The caller
    /// sends `LoopInput`s for user interactions (approvals, AskUser responses,
    /// cancellation).
    pub fn run(
        self,
        conversation: Vec<ModelMessage>,
        options: CallOptions,
    ) -> (
        mpsc::UnboundedReceiver<LoopEvent>,
        mpsc::UnboundedSender<LoopInput>,
    ) {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (input_tx, input_rx) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            self.run_inner(conversation, options, event_tx, input_rx)
                .await;
        });

        (event_rx, input_tx)
    }

    async fn run_inner(
        self,
        mut conversation: Vec<ModelMessage>,
        options: CallOptions,
        event_tx: mpsc::UnboundedSender<LoopEvent>,
        mut input_rx: mpsc::UnboundedReceiver<LoopInput>,
    ) {
        let OrchestratorServices {
            ai_client,
            tool_registry,
            process_registry,
            db_path,
            skills_manager,
        } = self.services;

        let OrchestratorConfig {
            session_id,
            working_dir,
            permission_mode,
            max_iterations,
            user_id,
            initial_work_mode,
            generate_title,
        } = self.config;

        let mut work_mode = initial_work_mode;
        let mut last_token_count = 0usize;
        let mut exploration_budget_count = 0usize;
        let mut tool_failure_signatures: HashMap<String, usize> = HashMap::new();
        let mut title_generated = !generate_title;

        set_agent_state(&db_path, &session_id, "streaming");

        for iteration in 1..=max_iterations {
            // Build context-injected conversation
            let conversation_with_context = context::inject_context(
                &conversation,
                &db_path,
                &session_id,
                &working_dir,
                work_mode,
                &skills_manager,
            );

            // Stream AI response
            let api_rx = match ai_client
                .call_streaming(conversation_with_context, &options)
                .await
            {
                Ok(rx) => rx,
                Err(e) => {
                    let _ = event_tx.send(LoopEvent::Error {
                        error: format!("AI error: {}", e),
                    });
                    if last_token_count > 0 {
                        update_token_count(&db_path, &session_id, last_token_count);
                    }
                    set_agent_state(&db_path, &session_id, "error");
                    return;
                }
            };

            let result = stream::process_stream(api_rx, &event_tx).await;

            if result.total_tokens > 0 {
                last_token_count = result.total_tokens;
            }

            // Build and save assistant message
            let assistant_msg =
                build_assistant_message(&result.text, &result.thinking_blocks, &result.tool_calls);
            if !assistant_msg.content.is_empty() {
                conversation.push(assistant_msg.clone());
                save_message(&db_path, &session_id, &assistant_msg);
            }

            // Title generation on first response
            if !title_generated && !result.text.is_empty() {
                title_generated = true;
                let first_user_msg = conversation
                    .iter()
                    .find(|m| m.role == Role::User)
                    .and_then(|m| {
                        m.content.iter().find_map(|c| {
                            if let Content::Text { text } = c {
                                Some(text.clone())
                            } else {
                                None
                            }
                        })
                    })
                    .unwrap_or_default();

                if !first_user_msg.is_empty() {
                    let title_client = ai_client.clone();
                    let title_tx = event_tx.clone();
                    let title_session_id = session_id.clone();
                    let title_db_path = db_path.clone();
                    tokio::spawn(async move {
                        let title = ai_generate_title(&title_client, &first_user_msg).await;
                        if !title.is_empty() {
                            save_title(&title_db_path, &title_session_id, &title);
                            let _ = title_tx.send(LoopEvent::TitleGenerated { title });
                        }
                    });
                }
            }

            // No tool calls → check plan detection → finish turn
            if result.tool_calls.is_empty() {
                if work_mode == WorkMode::Plan
                    && handle_plan_detection(
                        &result.text,
                        &session_id,
                        &working_dir,
                        &db_path,
                        &event_tx,
                    )
                {
                    // Plan detected — emit events and return.
                    // The server's tool-result handler manages confirmation.
                    if last_token_count > 0 {
                        update_token_count(&db_path, &session_id, last_token_count);
                    }
                    set_agent_state(&db_path, &session_id, "awaiting_input");
                    let _ = event_tx.send(LoopEvent::Finished {
                        session_id: session_id.clone(),
                    });
                    return;
                }

                let _ = event_tx.send(LoopEvent::TurnComplete {
                    turn: iteration,
                    has_more: false,
                });
                break;
            }

            // AskUser partition
            let (ask_user_calls, non_ask_user_calls): (Vec<_>, Vec<_>) =
                result
                    .tool_calls
                    .iter()
                    .partition::<Vec<_>, _>(|t| t.name == "AskUserQuestion");

            if !ask_user_calls.is_empty() {
                let mut all_results: Vec<Content> = Vec::new();

                // Execute non-AskUser tools first
                if !non_ask_user_calls.is_empty() {
                    let other_calls: Vec<_> = non_ask_user_calls.into_iter().cloned().collect();
                    set_agent_state(&db_path, &session_id, "tool_executing");
                    let (other_results, _) = executor::execute_tools(
                        &other_calls,
                        &tool_registry,
                        &working_dir,
                        &process_registry,
                        &session_id,
                        &db_path,
                        user_id.as_deref(),
                        permission_mode,
                        work_mode,
                        &event_tx,
                        &mut input_rx,
                    )
                    .await;
                    all_results.extend(other_results);
                }

                // Add placeholder results for AskUser calls
                for call in &ask_user_calls {
                    all_results.push(Content::ToolResult {
                        tool_use_id: call.id.clone(),
                        output: serde_json::Value::String("Awaiting user response...".to_string()),
                        is_error: None,
                    });
                }

                let tool_msg = ModelMessage {
                    role: Role::User,
                    content: all_results,
                };
                conversation.push(tool_msg.clone());
                save_message(&db_path, &session_id, &tool_msg);

                for call in &ask_user_calls {
                    let _ = event_tx.send(LoopEvent::AwaitingInput {
                        tool_call_id: call.id.clone(),
                        tool_name: call.name.clone(),
                    });
                }

                if last_token_count > 0 {
                    update_token_count(&db_path, &session_id, last_token_count);
                }
                set_agent_state(&db_path, &session_id, "awaiting_input");
                let _ = event_tx.send(LoopEvent::Finished {
                    session_id: session_id.clone(),
                });
                return;
            }

            // Exploration budget tracking
            let all_readonly = result
                .tool_calls
                .iter()
                .all(|t| matches!(t.name.as_str(), "read" | "glob" | "grep"));
            let has_action = result.tool_calls.iter().any(|t| {
                matches!(
                    t.name.as_str(),
                    "edit"
                        | "write"
                        | "bash"
                        | "build"
                        | "task_start"
                        | "task_complete"
                        | "add_subtask"
                        | "set_dependency"
                        | "set_work_mode"
                        | "enter_plan_mode"
                )
            });
            if has_action {
                exploration_budget_count = 0;
            } else if all_readonly {
                exploration_budget_count += result.tool_calls.len();
            }

            // Execute tools
            set_agent_state(&db_path, &session_id, "tool_executing");
            let (tool_results, next_work_mode) = executor::execute_tools(
                &result.tool_calls,
                &tool_registry,
                &working_dir,
                &process_registry,
                &session_id,
                &db_path,
                user_id.as_deref(),
                permission_mode,
                work_mode,
                &event_tx,
                &mut input_rx,
            )
            .await;
            work_mode = next_work_mode;

            // Failure detection
            let fail_diagnostic = failure::detect_repeated_failures(
                &mut tool_failure_signatures,
                &result.tool_calls,
                &tool_results,
            );

            // Exploration budget warnings
            if exploration_budget_count >= EXPLORATION_BUDGET_HARD {
                tracing::warn!(
                    exploration_budget_count,
                    "Exploration budget hard threshold reached"
                );
            } else if exploration_budget_count >= EXPLORATION_BUDGET_SOFT {
                tracing::info!(
                    exploration_budget_count,
                    "Exploration budget soft threshold reached"
                );
            }

            // Save tool results
            let tool_msg = ModelMessage {
                role: Role::User,
                content: tool_results,
            };
            conversation.push(tool_msg.clone());
            save_message(&db_path, &session_id, &tool_msg);

            // Check fail-fast
            if let Some(diagnostic) = fail_diagnostic {
                tracing::warn!(
                    iteration,
                    session_id = %session_id,
                    diagnostic = %diagnostic,
                    "Fail-fast: stopping repeated tool failure loop"
                );
                set_agent_state(&db_path, &session_id, "idle");
                let _ = event_tx.send(LoopEvent::Error { error: diagnostic });
                let _ = event_tx.send(LoopEvent::TurnComplete {
                    turn: iteration,
                    has_more: false,
                });
                break;
            }

            set_agent_state(&db_path, &session_id, "streaming");
            let _ = event_tx.send(LoopEvent::TurnComplete {
                turn: iteration,
                has_more: true,
            });
        }

        if last_token_count > 0 {
            update_token_count(&db_path, &session_id, last_token_count);
        }
        set_agent_state(&db_path, &session_id, "idle");

        let _ = event_tx.send(LoopEvent::Finished {
            session_id: session_id.clone(),
        });
    }
}

// ── Plan detection ─────────────────────────────────────────────────────

fn handle_plan_detection(
    text: &str,
    session_id: &str,
    working_dir: &Path,
    db_path: &std::path::Path,
    event_tx: &mpsc::UnboundedSender<LoopEvent>,
) -> bool {
    let Some(mut plan) = plan_handler::try_detect_plan(text) else {
        return false;
    };
    plan.plan_file.session_id = Some(session_id.to_string());
    plan.plan_file.working_dir = Some(working_dir.to_string_lossy().to_string());

    match PlanManager::new(db_path.to_path_buf()) {
        Ok(plan_manager) => {
            if let Err(e) = plan_manager.save_plan_for_session(session_id, &plan.plan_file) {
                tracing::warn!(
                    session_id = %session_id,
                    "Failed to save detected plan: {}", e
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                session_id = %session_id,
                "Failed to initialize plan manager for detected plan: {}", e
            );
        }
    }

    let tasks: Vec<PlanTaskInfo> = plan
        .tasks
        .iter()
        .map(|t| PlanTaskInfo {
            description: t.description.clone(),
            completed: t.completed,
        })
        .collect();

    let _ = event_tx.send(LoopEvent::PlanUpdate {
        tasks: tasks.clone(),
    });

    let tool_call_id = format!("plan-confirm-{}", uuid::Uuid::new_v4());
    let _ = event_tx.send(LoopEvent::PlanComplete {
        tool_call_id: tool_call_id.clone(),
        title: plan.title,
        task_count: tasks.len(),
    });

    let _ = event_tx.send(LoopEvent::AwaitingInput {
        tool_call_id,
        tool_name: "PlanConfirm".to_string(),
    });

    true
}

// ── DB helpers ─────────────────────────────────────────────────────────

use std::path::Path;

fn build_assistant_message(
    text: &str,
    thinking_blocks: &[ThinkingBlock],
    tool_calls: &[crate::ai::types::AiToolCall],
) -> ModelMessage {
    let mut content = Vec::with_capacity(
        thinking_blocks.len() + tool_calls.len() + usize::from(!text.is_empty()),
    );

    for block in thinking_blocks {
        content.push(Content::Thinking {
            thinking: block.thinking.clone(),
            signature: block.signature.clone(),
        });
    }

    if !text.is_empty() {
        content.push(Content::Text {
            text: text.to_string(),
        });
    }

    for call in tool_calls {
        content.push(Content::ToolUse {
            id: call.id.clone(),
            name: call.name.clone(),
            input: call.arguments.clone(),
        });
    }

    ModelMessage {
        role: Role::Assistant,
        content,
    }
}

fn save_message(db_path: &Path, session_id: &str, message: &ModelMessage) {
    let role = match message.role {
        Role::User => "user",
        Role::Assistant => "assistant",
        _ => return,
    };

    match serde_json::to_string(&message.content) {
        Ok(json) => match Database::new(db_path) {
            Ok(db) => {
                let session_manager = SessionManager::new(db);
                if let Err(e) = session_manager.save_message(session_id, role, &json) {
                    tracing::error!("Failed to save message: {}", e);
                }
            }
            Err(e) => tracing::error!("Failed to open database while saving message: {}", e),
        },
        Err(e) => tracing::error!("Failed to serialize message: {}", e),
    }
}

fn save_title(db_path: &Path, session_id: &str, title: &str) {
    match Database::new(db_path) {
        Ok(db) => {
            let session_manager = SessionManager::new(db);
            if let Err(e) = session_manager.update_session_title(session_id, title) {
                tracing::warn!(
                    session_id = %session_id,
                    "Failed to save title: {}", e
                );
            }
        }
        Err(e) => tracing::error!("Failed to open database while saving title: {}", e),
    }
}

fn set_agent_state(db_path: &Path, session_id: &str, state: &str) {
    match Database::new(db_path) {
        Ok(db) => {
            let session_manager = SessionManager::new(db);
            if let Err(e) = session_manager.set_agent_state(session_id, state) {
                tracing::warn!(
                    session_id = %session_id,
                    "Failed to set agent state '{state}': {}", e
                );
            }
        }
        Err(e) => tracing::error!("Failed to open database while setting agent state: {}", e),
    }
}

fn update_token_count(db_path: &Path, session_id: &str, token_count: usize) {
    match Database::new(db_path) {
        Ok(db) => {
            let session_manager = SessionManager::new(db);
            if let Err(e) = session_manager.update_token_count(session_id, token_count) {
                tracing::warn!(
                    session_id = %session_id,
                    "Failed to update token count: {}", e
                );
            }
        }
        Err(e) => tracing::error!("Failed to open database while updating token count: {}", e),
    }
}
