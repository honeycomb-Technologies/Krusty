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
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::{mpsc, RwLock};

use crate::ai::client::{AiClient, CallOptions};
use crate::ai::models::resolve_context_window;
use crate::ai::title::generate_title as ai_generate_title;
use crate::ai::types::{Content, ModelMessage, Role};
use crate::constants;
use crate::plan::PlanManager;
use crate::process::ProcessRegistry;
use crate::skills::SkillsManager;
use crate::storage::{
    Database, PartialAssistantState, ProjectSettings, RecoveryDecision, RecoveryNonResumableReason,
    RecoveryStatus, RecoveryToolCall, SessionManager, SessionRecoveryState, SessionType, WorkMode,
};
use crate::tools::registry::{PermissionMode, ToolRegistry};

use super::compaction::{CompactionManager, CompactionReason};
use super::context;
use super::context_ledger::{ContextLedger, ContinuationDecision};
use super::executor;
use super::failure;
use super::loop_events::{LoopEvent, LoopInput, LoopStopReason, PlanTaskInfo};
use super::plan_handler;
use super::stream::{self, ThinkingBlock};
use super::DelegatedProgressEvent;

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
            let mut conversation_with_context = inject_runtime_context(
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
            );
            let estimated_tokens_before =
                super::estimate_conversation_tokens(&conversation_with_context);

            if compaction_manager.should_compact(estimated_tokens_before) {
                let Some(outcome) = compaction_manager
                    .compact_conversation(&conversation, CompactionReason::ContextPressure)
                else {
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
                            "Live context compaction could not reduce the active conversation;{}",
                            continuation_hint
                        ),
                    });
                    if last_token_count > 0 {
                        update_token_count(&db_path, &session_id, last_token_count);
                    }
                    clear_recovery_state(&db_path, &session_id);
                    set_agent_state(&db_path, &session_id, "error");
                    let _ = event_tx.send(LoopEvent::Finished {
                        session_id: session_id.clone(),
                        stop_reason: LoopStopReason::ContextCompactionFailed,
                    });
                    return;
                };

                conversation = outcome.conversation;
                context_ledger.update_from_conversation(&conversation);
                context_ledger.record_compaction(
                    CompactionReason::ContextPressure,
                    estimated_tokens_before,
                    super::estimate_conversation_tokens(&conversation),
                    outcome.replaced_messages,
                );
                persist_context_state(&db_path, &session_id, &context_ledger);
                persist_conversation(&db_path, &session_id, &conversation);

                conversation_with_context = inject_runtime_context(
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
                );
                let estimated_tokens_after =
                    super::estimate_conversation_tokens(&conversation_with_context);
                last_token_count = estimated_tokens_after;
                update_token_count(&db_path, &session_id, estimated_tokens_after);

                let _ = event_tx.send(LoopEvent::ContextCompacted {
                    reason: CompactionReason::ContextPressure.as_str().to_string(),
                    estimated_tokens_before,
                    estimated_tokens_after,
                    replaced_messages: outcome.replaced_messages,
                });

                if estimated_tokens_after >= estimated_tokens_before
                    || compaction_manager.is_hard_failure(estimated_tokens_after)
                {
                    let _ = event_tx.send(LoopEvent::Error {
                        error: format!(
                            "Live context compaction reduced history from ~{} to ~{} tokens, which is still above the safe limit for this thread",
                            estimated_tokens_before, estimated_tokens_after
                        ),
                    });
                    clear_recovery_state(&db_path, &session_id);
                    set_agent_state(&db_path, &session_id, "error");
                    let _ = event_tx.send(LoopEvent::Finished {
                        session_id: session_id.clone(),
                        stop_reason: LoopStopReason::ContextCompactionFailed,
                    });
                    return;
                }
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
                    clear_recovery_state(&db_path, &session_id);
                    set_agent_state(&db_path, &session_id, "awaiting_input");
                    let _ = event_tx.send(LoopEvent::Finished {
                        session_id: session_id.clone(),
                        stop_reason: LoopStopReason::AwaitingInput,
                    });
                    return;
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
                clear_recovery_state(&db_path, &session_id);
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
            persist_recovery_state(
                &db_path,
                &session_id,
                &build_recovery_state(
                    &context_ledger,
                    RecoveryStatus::ToolExecuting,
                    None,
                    None,
                    build_partial_assistant_state(&result.recovery_checkpoint),
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

fn finalize_explore_only_turn(
    tool_calls: &[crate::ai::types::AiToolCall],
    tool_results: &[Content],
) -> Option<String> {
    if tool_calls.len() != 1 || tool_calls.first()?.name != "explore" {
        return None;
    }

    let Content::ToolResult {
        tool_use_id,
        output,
        is_error,
    } = tool_results.first()?
    else {
        return None;
    };

    if tool_use_id != &tool_calls.first()?.id || is_error.unwrap_or(false) {
        return None;
    }

    let parsed = match output {
        serde_json::Value::String(text) => serde_json::from_str::<serde_json::Value>(text).ok()?,
        other => other.clone(),
    };
    let payload = parsed.get("result").unwrap_or(&parsed);
    let outcome = payload
        .get("outcome")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let usable_agents = payload
        .get("usable_agents")
        .or_else(|| payload.get("successful_agents"))
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    if !matches!(outcome, "success" | "partial") || usable_agents == 0 {
        return None;
    }

    let message = payload
        .get("message")
        .and_then(|value| value.as_str())
        .unwrap_or("Explore completed.");
    if let Some(human_review) = payload
        .get("human_review")
        .and_then(|value| value.as_str())
        .filter(|review| !review.trim().is_empty())
    {
        return Some(human_review.to_string());
    }
    payload
        .get("investigation_summary")
        .and_then(|value| value.as_str())
        .filter(|summary| !summary.trim().is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            Some(message)
                .filter(|message| !message.trim().is_empty())
                .map(ToString::to_string)
        })
}

fn maybe_generate_title(
    conversation: &[ModelMessage],
    ai_client: &Arc<AiClient>,
    event_tx: &mpsc::UnboundedSender<LoopEvent>,
    session_id: &str,
    db_path: &Path,
) {
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

    if first_user_msg.is_empty() {
        return;
    }

    let title_client = ai_client.clone();
    let title_tx = event_tx.clone();
    let title_session_id = session_id.to_string();
    let title_db_path = db_path.to_path_buf();
    tokio::spawn(async move {
        let title = ai_generate_title(&title_client, &first_user_msg).await;
        if !title.is_empty() {
            save_title(&title_db_path, &title_session_id, &title);
            let _ = title_tx.send(LoopEvent::TitleGenerated { title });
        }
    });
}

fn save_message(db_path: &Path, session_id: &str, message: &ModelMessage) {
    let role = match message.role {
        Role::User => "user",
        Role::Assistant => "assistant",
        _ => return,
    };

    match serde_json::to_string(&message.content) {
        Ok(json) => {
            let Some(session_manager) = open_session_manager(db_path, "saving message") else {
                return;
            };
            if let Err(e) = session_manager.save_message(session_id, role, &json) {
                tracing::error!("Failed to save message: {}", e);
            }
        }
        Err(e) => tracing::error!("Failed to serialize message: {}", e),
    }
}

fn persist_conversation(db_path: &Path, session_id: &str, conversation: &[ModelMessage]) {
    let persisted = conversation
        .iter()
        .filter_map(|message| {
            let role = match message.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => return None,
            };

            serde_json::to_string(&message.content)
                .ok()
                .map(|content| (role.to_string(), content))
        })
        .collect::<Vec<_>>();

    let Some(session_manager) = open_session_manager(db_path, "persisting compacted conversation")
    else {
        return;
    };
    if let Err(e) = session_manager.replace_session_messages(session_id, &persisted) {
        tracing::error!("Failed to persist compacted conversation: {}", e);
    }
}

fn persist_context_state(db_path: &Path, session_id: &str, ledger: &ContextLedger) {
    let ledger_json = match serde_json::to_string(&ledger.persistence_record()) {
        Ok(json) => json,
        Err(e) => {
            tracing::warn!(
                session_id = %session_id,
                "Failed to serialize context ledger snapshot: {}", e
            );
            return;
        }
    };

    let continuation_json = match serde_json::to_string(&ledger.continuation_contract()) {
        Ok(json) => json,
        Err(e) => {
            tracing::warn!(
                session_id = %session_id,
                "Failed to serialize continuation contract: {}", e
            );
            return;
        }
    };

    let Some(session_manager) =
        open_session_manager(db_path, "persisting context continuation state")
    else {
        return;
    };
    if let Err(e) = session_manager.update_context_continuation_state(
        session_id,
        &ledger_json,
        &continuation_json,
    ) {
        tracing::warn!(
            session_id = %session_id,
            "Failed to persist context continuation state: {}", e
        );
    }
}

fn persist_recovery_state(db_path: &Path, session_id: &str, recovery: &SessionRecoveryState) {
    update_recovery_state(db_path, session_id, Some(recovery));
}

fn clear_recovery_state(db_path: &Path, session_id: &str) {
    update_recovery_state(db_path, session_id, None);
}

fn save_title(db_path: &Path, session_id: &str, title: &str) {
    let Some(session_manager) = open_session_manager(db_path, "saving title") else {
        return;
    };
    if let Err(e) = session_manager.update_session_title(session_id, title) {
        tracing::warn!(
            session_id = %session_id,
            "Failed to save title: {}", e
        );
    }
}

fn set_agent_state(db_path: &Path, session_id: &str, state: &str) {
    let Some(session_manager) = open_session_manager(db_path, "setting agent state") else {
        return;
    };
    if let Err(e) = session_manager.set_agent_state(session_id, state) {
        tracing::warn!(
            session_id = %session_id,
            "Failed to set agent state '{state}': {}", e
        );
    }
}

fn update_token_count(db_path: &Path, session_id: &str, token_count: usize) {
    let Some(session_manager) = open_session_manager(db_path, "updating token count") else {
        return;
    };
    if let Err(e) = session_manager.update_token_count(session_id, token_count) {
        tracing::warn!(
            session_id = %session_id,
            "Failed to update token count: {}", e
        );
    }
}

fn update_recovery_state(
    db_path: &Path,
    session_id: &str,
    recovery: Option<&SessionRecoveryState>,
) {
    let action = if recovery.is_some() {
        "persisting recovery state"
    } else {
        "clearing recovery state"
    };
    let Some(session_manager) = open_session_manager(db_path, action) else {
        return;
    };

    let result = match recovery {
        Some(recovery) => session_manager.update_recovery_state(session_id, recovery),
        None => session_manager.clear_recovery_state(session_id),
    };
    if let Err(e) = result {
        let verb = if recovery.is_some() {
            "persist"
        } else {
            "clear"
        };
        tracing::warn!(
            session_id = %session_id,
            "Failed to {verb} recovery state: {}", e
        );
    }
}

fn open_session_manager(db_path: &Path, action: &str) -> Option<SessionManager> {
    match Database::new(db_path) {
        Ok(db) => Some(SessionManager::new(db)),
        Err(e) => {
            tracing::error!("Failed to open database while {}: {}", action, e);
            None
        }
    }
}

fn continuation_recovery_message(ledger: &ContextLedger) -> String {
    match ledger.continuation_decision() {
        ContinuationDecision::Resumable {
            latest_user_objective,
        } => format!(
            "Continuation contract: resumable. Preserved objective: {}",
            latest_user_objective
        ),
        ContinuationDecision::NonResumable { reason } => format!(
            "Continuation contract: non-resumable ({:?}). Start a fresh turn with explicit user intent.",
            reason
        ),
    }
}

fn build_partial_assistant_state(checkpoint: &stream::StreamCheckpoint) -> PartialAssistantState {
    PartialAssistantState {
        text: checkpoint.text.clone(),
        thinking: checkpoint.thinking.clone(),
        tool_calls: checkpoint
            .tool_calls
            .iter()
            .map(|tool| RecoveryToolCall {
                id: tool.id.clone(),
                name: tool.name.clone(),
            })
            .collect(),
    }
}

fn build_recovery_state(
    ledger: &ContextLedger,
    status: RecoveryStatus,
    stop_reason: Option<LoopStopReason>,
    last_error: Option<String>,
    partial_assistant: PartialAssistantState,
) -> SessionRecoveryState {
    SessionRecoveryState::new(
        status.clone(),
        stop_reason,
        last_error,
        partial_assistant.clone(),
        recovery_decision(ledger, &status, &partial_assistant),
    )
}

fn recovery_decision(
    ledger: &ContextLedger,
    status: &RecoveryStatus,
    partial_assistant: &PartialAssistantState,
) -> RecoveryDecision {
    let override_reason = match status {
        RecoveryStatus::ToolExecuting => Some(RecoveryNonResumableReason::ToolExecutionInProgress),
        RecoveryStatus::Streaming | RecoveryStatus::Interrupted
            if !partial_assistant.tool_calls.is_empty() =>
        {
            Some(RecoveryNonResumableReason::PendingToolCall)
        }
        _ => None,
    };

    if let Some(reason) = override_reason {
        return RecoveryDecision::NonResumable { reason };
    }

    match ledger.continuation_decision() {
        ContinuationDecision::Resumable {
            latest_user_objective,
        } => RecoveryDecision::Resumable {
            latest_user_objective,
        },
        ContinuationDecision::NonResumable { reason } => RecoveryDecision::NonResumable {
            reason: match reason {
                super::context_ledger::NonResumableReason::MissingUserObjective => {
                    RecoveryNonResumableReason::MissingUserObjective
                }
                super::context_ledger::NonResumableReason::EmptyConversation => {
                    RecoveryNonResumableReason::EmptyConversation
                }
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{finalize_explore_only_turn, inject_runtime_context};
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
