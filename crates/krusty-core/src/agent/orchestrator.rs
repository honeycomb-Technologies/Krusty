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

mod message_builder;
mod persistence;
mod plan_flow;
mod recovery;
mod title;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::{mpsc, RwLock};

use crate::ai::client::{AiClient, CallOptions};
use crate::ai::models::resolve_context_window;
use crate::ai::types::{Content, ModelMessage, Role};
use crate::constants;
use crate::process::ProcessRegistry;
use crate::skills::SkillsManager;
use crate::storage::{
    PartialAssistantState, PendingInteractionSnapshot, ProjectSettings, RecoveryStatus,
    SessionManager, SessionType, WorkMode,
};
use crate::tools::registry::{PermissionMode, ToolRegistry};

use super::compaction::CompactionManager;
use super::context;
use super::context_ledger::{ContextLedger, ContinuationDecision};
use super::executor;
use super::failure;
use super::loop_events::{LoopEvent, LoopInput, LoopStopReason};
use super::stream;
use super::DelegatedProgressEvent;
use super::{create_pinched_session, CreatePinchedSessionRequest};

use self::message_builder::{build_assistant_message, finalize_explore_only_turn};
use self::persistence::{
    clear_recovery_state, persist_context_state, persist_recovery_state, save_message,
    set_agent_state, update_token_count,
};
use self::plan_flow::handle_plan_detection;
use self::recovery::{
    build_awaiting_input_recovery_state, build_partial_assistant_state, build_recovery_state,
    continuation_recovery_message,
};
use self::title::maybe_generate_title;

const EXPLORATION_BUDGET_SOFT: usize = 15;
const EXPLORATION_BUDGET_HARD: usize = 30;

/// Configuration for an orchestrator run.
pub struct OrchestratorConfig {
    pub session_id: String,
    pub working_dir: PathBuf,
    pub project_dir: Option<PathBuf>,
    pub mako_crew_slug: Option<String>,
    pub session_type: SessionType,
    pub permission_mode: PermissionMode,
    pub max_iterations: Option<usize>,
    pub stream_idle_timeout: std::time::Duration,
    pub user_id: Option<String>,
    pub initial_work_mode: WorkMode,
    /// Whether to generate a title on first AI response.
    /// Set to true for new sessions, false for resumed conversations.
    pub generate_title: bool,
    /// Optional explore delegated progress channel for external surfaces.
    pub delegated_progress_tx: Option<mpsc::UnboundedSender<DelegatedProgressEvent>>,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            working_dir: PathBuf::new(),
            project_dir: None,
            mako_crew_slug: None,
            session_type: SessionType::Code,
            permission_mode: PermissionMode::default(),
            max_iterations: None,
            stream_idle_timeout: constants::http::STREAM_TIMEOUT,
            user_id: None,
            initial_work_mode: WorkMode::default(),
            generate_title: false,
            delegated_progress_tx: None,
        }
    }
}

/// Shared services the orchestrator needs.
#[derive(Clone)]
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

fn session_type_name(session_type: SessionType) -> &'static str {
    match session_type {
        SessionType::Chat => "chat",
        SessionType::Code => "code",
        SessionType::Mako => "mako",
    }
}

fn inject_runtime_context(
    conversation: &[ModelMessage],
    db_path: &Path,
    session_id: &str,
    working_dir: &Path,
    project_dir: Option<&Path>,
    mako_crew_slug: Option<&str>,
    work_mode: WorkMode,
    skills_manager: &RwLock<SkillsManager>,
    model: Option<&str>,
    session_type: SessionType,
    user_id: Option<&str>,
) -> Vec<ModelMessage> {
    context::inject_context(
        conversation,
        db_path,
        session_id,
        working_dir,
        project_dir,
        work_mode,
        skills_manager,
        model,
        Some(session_type_name(session_type)),
        mako_crew_slug,
        user_id,
    )
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
        let (trace_tx, trace_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        let trace_db_path = self.services.db_path.clone();
        let trace_session_id = self.config.session_id.clone();
        let trace_run_id = super::observability::new_runtime_trace_run_id();

        tokio::spawn(async move {
            super::observability::forward_runtime_traces(
                trace_db_path,
                trace_session_id,
                trace_run_id,
                trace_rx,
                event_tx,
            )
            .await;
        });

        tokio::spawn(async move {
            self.run_inner(conversation, options, trace_tx, input_rx)
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
            project_dir,
            mako_crew_slug,
            session_type,
            permission_mode,
            max_iterations,
            stream_idle_timeout,
            user_id,
            initial_work_mode,
            generate_title,
            delegated_progress_tx,
        } = self.config;

        // Load per-project settings from .krusty/settings.json
        let project_settings = project_dir
            .as_deref()
            .map(ProjectSettings::load)
            .unwrap_or_default();

        // Apply permission_mode override from project settings
        let permission_mode = if let Some(ref mode_str) = project_settings.permission_mode {
            match mode_str.as_str() {
                "autonomous" => {
                    tracing::info!("Project settings override: permission_mode = autonomous");
                    PermissionMode::Autonomous
                }
                "supervised" => {
                    tracing::info!("Project settings override: permission_mode = supervised");
                    PermissionMode::Supervised
                }
                other => {
                    tracing::warn!(
                        "Unknown permission_mode in project settings: {:?}, keeping default",
                        other
                    );
                    permission_mode
                }
            }
        } else {
            permission_mode
        };

        // Log model override (consumed by the presentation layer that constructs AiClient)
        if let Some(ref model) = project_settings.model {
            tracing::info!(
                "Project settings specify model override: {} (active client model: {})",
                model,
                ai_client.config().model,
            );
        }

        let mut work_mode = initial_work_mode;
        let mut last_token_count = 0usize;
        let mut exploration_budget_count = 0usize;
        let mut tool_failure_signatures: HashMap<String, usize> = HashMap::new();
        let mut tool_pattern_signatures: HashMap<String, usize> = HashMap::new();
        let mut title_generated = !generate_title;
        let mut iteration = 0usize;
        let model_context_window = resolve_context_window(
            ai_client.provider_id(),
            &ai_client.config().model,
            ai_client.config().api_format,
        );
        let compaction_manager = CompactionManager::for_model(
            ai_client.provider_id(),
            ai_client.config().api_format,
            &ai_client.config().model,
            model_context_window,
        );
        let mut context_ledger = ContextLedger::from_conversation(&conversation);
        persist_context_state(&db_path, &session_id, &context_ledger);
        clear_recovery_state(&db_path, &session_id);

        set_agent_state(&db_path, &session_id, "streaming");

        loop {
            if max_iterations.is_some_and(|max| iteration >= max) {
                let message = max_iterations
                    .map(|max| format!("Agent turn budget exhausted after {} turns", max))
                    .unwrap_or_else(|| "Agent turn budget exhausted".to_string());
                let _ = event_tx.send(LoopEvent::Error { error: message });
                if last_token_count > 0 {
                    update_token_count(&db_path, &session_id, last_token_count);
                }
                clear_recovery_state(&db_path, &session_id);
                set_agent_state(&db_path, &session_id, "idle");
                let _ = event_tx.send(LoopEvent::Finished {
                    session_id: session_id.clone(),
                    stop_reason: LoopStopReason::BudgetExhausted,
                });
                return;
            }
            iteration += 1;

            // Build context-injected conversation
            let conversation_with_context = inject_runtime_context(
                &conversation,
                &db_path,
                &session_id,
                &working_dir,
                project_dir.as_deref(),
                mako_crew_slug.as_deref(),
                work_mode,
                &skills_manager,
                Some(ai_client.config().model.as_str()),
                session_type,
                user_id.as_deref(),
            );
            let estimated_tokens_before =
                super::estimate_conversation_tokens(&conversation_with_context);

            if compaction_manager.should_compact(estimated_tokens_before) {
                let session_info = match crate::storage::Database::new(&db_path) {
                    Ok(db) => SessionManager::new(db)
                        .get_session(&session_id)
                        .ok()
                        .flatten(),
                    Err(error) => {
                        tracing::warn!(
                            session_id = %session_id,
                            error = %error,
                            "Failed to open database while preparing automatic pinch"
                        );
                        None
                    }
                };
                let source_session_title = session_info
                    .as_ref()
                    .map(|session| session.title.as_str())
                    .unwrap_or("Session");
                let target_branch = session_info
                    .as_ref()
                    .and_then(|session| session.target_branch.as_deref());
                let model_for_child = session_info
                    .as_ref()
                    .and_then(|session| session.model.as_deref())
                    .unwrap_or(ai_client.config().model.as_str());

                let pinch_result = match create_pinched_session(CreatePinchedSessionRequest {
                    db_path: &db_path,
                    ai_client: Some(ai_client.as_ref()),
                    session_id: &session_id,
                    source_session_title,
                    conversation: &conversation,
                    working_dir: &working_dir,
                    model: Some(model_for_child),
                    target_branch,
                    preservation_hints: None,
                    direction: None,
                    initial_user_message: Some("Continue working on the current task.".to_string()),
                })
                .await
                {
                    Ok(result) => result,
                    Err(error) => {
                        let continuation = context_ledger.continuation_decision();
                        persist_context_state(&db_path, &session_id, &context_ledger);
                        let continuation_hint = match continuation {
                            ContinuationDecision::Resumable {
                                latest_user_objective,
                            } => format!(
                                " continuation candidate preserved objective: {}",
                                latest_user_objective
                            ),
                            ContinuationDecision::NonResumable { reason } => {
                                format!(" continuation is non-resumable: {:?}", reason)
                            }
                        };
                        let _ = event_tx.send(LoopEvent::Error {
                            error: format!(
                                "Automatic pinch could not create a continuation session;{} ({})",
                                continuation_hint, error
                            ),
                        });
                        if last_token_count > 0 {
                            update_token_count(&db_path, &session_id, last_token_count);
                        }
                        clear_recovery_state(&db_path, &session_id);
                        set_agent_state(&db_path, &session_id, "error");
                        let _ = event_tx.send(LoopEvent::Finished {
                            session_id: session_id.clone(),
                            stop_reason: LoopStopReason::PinchFailed,
                        });
                        return;
                    }
                };

                persist_context_state(&db_path, &session_id, &context_ledger);
                clear_recovery_state(&db_path, &session_id);
                set_agent_state(&db_path, &session_id, "idle");
                let _ = event_tx.send(LoopEvent::SessionPinched {
                    reason: "context_pressure".to_string(),
                    source_session_id: session_id.clone(),
                    new_session_id: pinch_result.new_session_id,
                    estimated_tokens_before,
                });
                let _ = event_tx.send(LoopEvent::Finished {
                    session_id: session_id.clone(),
                    stop_reason: LoopStopReason::Pinched,
                });
                return;
            }

            // Stream AI response
            persist_recovery_state(
                &db_path,
                &session_id,
                &build_recovery_state(
                    &context_ledger,
                    RecoveryStatus::Streaming,
                    None,
                    None,
                    PartialAssistantState::default(),
                ),
            );
            let api_rx = match ai_client
                .call_streaming(conversation_with_context, &options)
                .await
            {
                Ok(rx) => rx,
                Err(e) => {
                    let error = format!("AI error: {}", e);
                    persist_recovery_state(
                        &db_path,
                        &session_id,
                        &build_recovery_state(
                            &context_ledger,
                            RecoveryStatus::Interrupted,
                            Some(LoopStopReason::ProviderError),
                            Some(error.clone()),
                            PartialAssistantState::default(),
                        ),
                    );
                    let _ = event_tx.send(LoopEvent::Error { error });
                    if last_token_count > 0 {
                        update_token_count(&db_path, &session_id, last_token_count);
                    }
                    set_agent_state(&db_path, &session_id, "error");
                    let _ = event_tx.send(LoopEvent::Finished {
                        session_id: session_id.clone(),
                        stop_reason: LoopStopReason::ProviderError,
                    });
                    return;
                }
            };

            let result =
                stream::process_stream(api_rx, &event_tx, stream_idle_timeout, |checkpoint| {
                    persist_recovery_state(
                        &db_path,
                        &session_id,
                        &build_recovery_state(
                            &context_ledger,
                            RecoveryStatus::Streaming,
                            None,
                            None,
                            build_partial_assistant_state(checkpoint),
                        ),
                    );
                })
                .await;

            if result.total_tokens > 0 {
                last_token_count = result.total_tokens;
            }

            if let Some(stop_reason) = result.stop_reason.clone() {
                persist_recovery_state(
                    &db_path,
                    &session_id,
                    &build_recovery_state(
                        &context_ledger,
                        RecoveryStatus::Interrupted,
                        Some(stop_reason.clone()),
                        result.last_error.clone(),
                        build_partial_assistant_state(&result.recovery_checkpoint),
                    ),
                );
                persist_context_state(&db_path, &session_id, &context_ledger);
                let _ = event_tx.send(LoopEvent::Error {
                    error: continuation_recovery_message(&context_ledger),
                });
                if last_token_count > 0 {
                    update_token_count(&db_path, &session_id, last_token_count);
                }
                set_agent_state(&db_path, &session_id, "error");
                let _ = event_tx.send(LoopEvent::Finished {
                    session_id: session_id.clone(),
                    stop_reason,
                });
                return;
            }

            // Build and save assistant message
            let assistant_msg =
                build_assistant_message(&result.text, &result.thinking_blocks, &result.tool_calls);
            if !assistant_msg.content.is_empty() {
                conversation.push(assistant_msg.clone());
                context_ledger.update_from_conversation(&conversation);
                persist_context_state(&db_path, &session_id, &context_ledger);
                save_message(&db_path, &session_id, &assistant_msg);
            }

            // Title generation on first response
            if !title_generated && !result.text.is_empty() {
                title_generated = true;
                maybe_generate_title(&conversation, &ai_client, &event_tx, &session_id, &db_path);
            }

            // No tool calls → check plan detection → finish turn
            if result.tool_calls.is_empty() {
                if work_mode == WorkMode::Plan {
                    if let Some(pending_interaction) = handle_plan_detection(
                        &result.text,
                        &session_id,
                        &working_dir,
                        &db_path,
                        &event_tx,
                    ) {
                        // Plan detected — emit events, persist the pending confirmation snapshot, and return.
                        // The server's tool-result handler manages confirmation.
                        if last_token_count > 0 {
                            update_token_count(&db_path, &session_id, last_token_count);
                        }
                        persist_recovery_state(
                            &db_path,
                            &session_id,
                            &build_awaiting_input_recovery_state(
                                build_partial_assistant_state(&result.recovery_checkpoint),
                                vec![pending_interaction],
                            ),
                        );
                        set_agent_state(&db_path, &session_id, "awaiting_input");
                        let _ = event_tx.send(LoopEvent::Finished {
                            session_id: session_id.clone(),
                            stop_reason: LoopStopReason::AwaitingInput,
                        });
                        return;
                    }
                }

                let _ = event_tx.send(LoopEvent::TurnComplete {
                    turn: iteration,
                    has_more: false,
                });
                if last_token_count > 0 {
                    update_token_count(&db_path, &session_id, last_token_count);
                }
                clear_recovery_state(&db_path, &session_id);
                set_agent_state(&db_path, &session_id, "idle");
                let _ = event_tx.send(LoopEvent::Finished {
                    session_id: session_id.clone(),
                    stop_reason: LoopStopReason::Completed,
                });
                return;
            }

            // AskUser partition
            let (ask_user_calls, non_ask_user_calls): (Vec<_>, Vec<_>) =
                result
                    .tool_calls
                    .iter()
                    .partition::<Vec<_>, _>(|t| t.name == "AskUserQuestion");

            if !ask_user_calls.is_empty() {
                let ask_user_partial_assistant =
                    build_partial_assistant_state(&result.recovery_checkpoint);
                let ask_user_pending_interactions = ask_user_calls
                    .iter()
                    .map(|call| {
                        PendingInteractionSnapshot::ask_user_from_call(&call.id, &call.arguments)
                    })
                    .collect::<Vec<_>>();
                let mut all_results: Vec<Content> = Vec::new();

                // Execute non-AskUser tools first
                if !non_ask_user_calls.is_empty() {
                    let other_calls: Vec<_> = non_ask_user_calls.into_iter().cloned().collect();
                    set_agent_state(&db_path, &session_id, "tool_executing");
                    let (other_results, _) = executor::execute_tools(
                        &other_calls,
                        &tool_registry,
                        &ai_client,
                        &working_dir,
                        project_dir.as_deref(),
                        &process_registry,
                        &session_id,
                        &db_path,
                        user_id.as_deref(),
                        permission_mode,
                        work_mode,
                        Some(&ask_user_partial_assistant),
                        delegated_progress_tx.as_ref(),
                        &event_tx,
                        &mut input_rx,
                        project_settings.subagent_max_turns,
                        project_settings.disabled_tools.as_deref(),
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
                context_ledger.update_from_conversation(&conversation);
                persist_context_state(&db_path, &session_id, &context_ledger);
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
                persist_recovery_state(
                    &db_path,
                    &session_id,
                    &build_awaiting_input_recovery_state(
                        ask_user_partial_assistant,
                        ask_user_pending_interactions,
                    ),
                );
                set_agent_state(&db_path, &session_id, "awaiting_input");
                let _ = event_tx.send(LoopEvent::Finished {
                    session_id: session_id.clone(),
                    stop_reason: LoopStopReason::AwaitingInput,
                });
                return;
            }

            if let Some(diagnostic) = failure::detect_repeated_read_only_sequence(
                &mut tool_pattern_signatures,
                &result.tool_calls,
            ) {
                tracing::warn!(
                    iteration,
                    session_id = %session_id,
                    diagnostic = %diagnostic,
                    "Loop guard: repeated read-only exploration sequence"
                );
                if last_token_count > 0 {
                    update_token_count(&db_path, &session_id, last_token_count);
                }
                clear_recovery_state(&db_path, &session_id);
                set_agent_state(&db_path, &session_id, "idle");
                let _ = event_tx.send(LoopEvent::Error { error: diagnostic });
                let _ = event_tx.send(LoopEvent::TurnComplete {
                    turn: iteration,
                    has_more: false,
                });
                let _ = event_tx.send(LoopEvent::Finished {
                    session_id: session_id.clone(),
                    stop_reason: LoopStopReason::LoopGuardTriggered,
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
            let tool_execution_partial_assistant =
                build_partial_assistant_state(&result.recovery_checkpoint);
            persist_recovery_state(
                &db_path,
                &session_id,
                &build_recovery_state(
                    &context_ledger,
                    RecoveryStatus::ToolExecuting,
                    None,
                    None,
                    tool_execution_partial_assistant.clone(),
                ),
            );
            set_agent_state(&db_path, &session_id, "tool_executing");
            let (tool_results, next_work_mode) = executor::execute_tools(
                &result.tool_calls,
                &tool_registry,
                &ai_client,
                &working_dir,
                project_dir.as_deref(),
                &process_registry,
                &session_id,
                &db_path,
                user_id.as_deref(),
                permission_mode,
                work_mode,
                Some(&tool_execution_partial_assistant),
                delegated_progress_tx.as_ref(),
                &event_tx,
                &mut input_rx,
                project_settings.subagent_max_turns,
                project_settings.disabled_tools.as_deref(),
            )
            .await;
            work_mode = next_work_mode;

            // Failure detection
            let fail_diagnostic = failure::detect_repeated_failures(
                &mut tool_failure_signatures,
                &result.tool_calls,
                &tool_results,
            );
            let explore_diagnostic =
                failure::detect_terminal_explore_failure(&result.tool_calls, &tool_results);

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
            context_ledger.update_from_conversation(&conversation);
            persist_context_state(&db_path, &session_id, &context_ledger);
            save_message(&db_path, &session_id, &tool_msg);
            clear_recovery_state(&db_path, &session_id);

            // Check fail-fast
            if let Some(diagnostic) = fail_diagnostic.or(explore_diagnostic) {
                tracing::warn!(
                    iteration,
                    session_id = %session_id,
                    diagnostic = %diagnostic,
                    "Fail-fast: stopping repeated tool failure loop"
                );
                clear_recovery_state(&db_path, &session_id);
                set_agent_state(&db_path, &session_id, "idle");
                let _ = event_tx.send(LoopEvent::Error { error: diagnostic });
                let _ = event_tx.send(LoopEvent::TurnComplete {
                    turn: iteration,
                    has_more: false,
                });
                let _ = event_tx.send(LoopEvent::Finished {
                    session_id: session_id.clone(),
                    stop_reason: LoopStopReason::LoopGuardTriggered,
                });
                return;
            }

            if let Some(explore_summary) =
                finalize_explore_only_turn(&result.tool_calls, &tool_msg.content)
            {
                let _ = event_tx.send(LoopEvent::TextDelta {
                    delta: explore_summary.clone(),
                });

                let assistant_msg = ModelMessage {
                    role: Role::Assistant,
                    content: vec![Content::Text {
                        text: explore_summary,
                    }],
                };
                conversation.push(assistant_msg.clone());
                context_ledger.update_from_conversation(&conversation);
                persist_context_state(&db_path, &session_id, &context_ledger);
                save_message(&db_path, &session_id, &assistant_msg);

                if !title_generated {
                    maybe_generate_title(
                        &conversation,
                        &ai_client,
                        &event_tx,
                        &session_id,
                        &db_path,
                    );
                }

                if last_token_count > 0 {
                    update_token_count(&db_path, &session_id, last_token_count);
                }
                clear_recovery_state(&db_path, &session_id);
                set_agent_state(&db_path, &session_id, "idle");
                let _ = event_tx.send(LoopEvent::TurnComplete {
                    turn: iteration,
                    has_more: false,
                });
                let _ = event_tx.send(LoopEvent::Finished {
                    session_id: session_id.clone(),
                    stop_reason: LoopStopReason::Completed,
                });
                return;
            }

            set_agent_state(&db_path, &session_id, "streaming");
            let _ = event_tx.send(LoopEvent::TurnComplete {
                turn: iteration,
                has_more: true,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::inject_runtime_context;
    use super::message_builder::finalize_explore_only_turn;
    use crate::ai::types::{AiToolCall, Content, ModelMessage, Role};
    use crate::skills::SkillsManager;
    use crate::storage::{SessionType, WorkMode};
    use serde_json::json;
    use tempfile::TempDir;
    use tokio::sync::RwLock;

    #[test]
    fn finalize_explore_only_turn_returns_summary_for_successful_explore() {
        let tool_calls = vec![AiToolCall {
            id: "call-1".to_string(),
            name: "explore".to_string(),
            arguments: json!({}),
        }];
        let tool_results = vec![Content::ToolResult {
            tool_use_id: "call-1".to_string(),
            output: json!({
                "tool": "explore",
                "result": {
                    "outcome": "success",
                    "usable_agents": 2,
                    "message": "Explore completed: 2 agents",
                    "human_review": "Architecture review completed across 2 targets.\n\nTarget reviews:\n- agent: Owns orchestration.\n- ai: Owns providers.",
                    "investigation_summary": "Delegated exploration gathered usable evidence for dir-0, dir-1.",
                    "confidence": "high",
                    "paths_examined_count": 15
                }
            }),
            is_error: None,
        }];

        let summary = finalize_explore_only_turn(&tool_calls, &tool_results)
            .expect("explore should finalize");

        assert!(summary.contains("Architecture review completed across 2 targets."));
        assert!(summary.contains("agent: Owns orchestration."));
        assert!(!summary.contains("Evidence examined: 15 tracked paths/files."));
    }

    #[test]
    fn finalize_explore_only_turn_skips_failed_or_non_explore_results() {
        let tool_calls = vec![AiToolCall {
            id: "call-1".to_string(),
            name: "explore".to_string(),
            arguments: json!({}),
        }];
        let tool_results = vec![Content::ToolResult {
            tool_use_id: "call-1".to_string(),
            output: json!({
                "tool": "explore",
                "result": {
                    "outcome": "failed",
                    "usable_agents": 0
                }
            }),
            is_error: None,
        }];

        assert!(finalize_explore_only_turn(&tool_calls, &tool_results).is_none());
    }

    #[test]
    fn inject_runtime_context_applies_mako_session_identity() {
        let temp = TempDir::new().expect("temp dir should exist");
        let repo = temp.path();
        fs::create_dir_all(repo.join(".git")).expect("git dir should exist");
        fs::write(repo.join("AGENTS.md"), "repo instructions").expect("agents should exist");
        fs::write(repo.join("MAKO.md"), "Always Swimming.").expect("mako identity should exist");

        let skills = RwLock::new(SkillsManager::with_defaults(repo));
        let conversation = vec![ModelMessage {
            role: Role::User,
            content: vec![Content::Text {
                text: "hello".to_string(),
            }],
        }];
        let db_path = repo.join("krusty.db");

        let mako_injected = inject_runtime_context(
            &conversation,
            &db_path,
            "session-id",
            repo,
            Some(repo),
            None,
            WorkMode::Build,
            &skills,
            None,
            SessionType::Mako,
            None,
        );
        let code_injected = inject_runtime_context(
            &conversation,
            &db_path,
            "session-id",
            repo,
            Some(repo),
            None,
            WorkMode::Build,
            &skills,
            None,
            SessionType::Code,
            None,
        );

        assert!(mako_injected.iter().any(|message| {
            matches!(
                &message.content[0],
                Content::Text { text }
                    if text.contains("[MAKO PROJECT OVERLAY - MAKO.md]") && text.contains("Always Swimming.")
            )
        }));
        assert!(!code_injected.iter().any(|message| {
            matches!(
                &message.content[0],
                Content::Text { text } if text.contains("[MAKO PROJECT OVERLAY - MAKO.md]")
            )
        }));
    }
}
