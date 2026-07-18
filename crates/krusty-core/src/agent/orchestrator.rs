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
use std::time::Instant;

use tokio::sync::{mpsc, RwLock};

use crate::ai::client::{AiClient, CallOptions};
use crate::ai::model_profile::ModelProfile;
use crate::ai::models::resolve_context_window;
use crate::ai::retry::is_retryable_error_message;
use crate::ai::types::{AiToolCall, Content, ModelMessage, Role};
use crate::constants;
use crate::process::ProcessRegistry;
use crate::skills::SkillsManager;
use crate::storage::{
    Database, PartialAssistantState, PendingInteractionSnapshot, ProjectSettings, RecoveryStatus,
    SessionManager, SessionType, WorkMode,
};
use crate::tools::registry::effective_tool_call;
use crate::tools::registry::{FileObservationTracker, PermissionMode, ToolRegistry};

use super::compaction::{
    effective_context_window_for_runtime, is_context_overflow_error,
    microcompact::microcompact_messages, run_compaction_pipeline_observed, CompactionManager,
    CompactionRequest, CompactionRequestBudget, CompactionResult, CompactionTrigger,
};
use super::context;
use super::context_ledger::ContextLedger;
use super::executor;
use super::failure;
use super::loop_events::{LoopEvent, LoopInput, LoopInputInbox, LoopStopReason};
use super::stream;
use super::DelegatedProgressEvent;

use self::message_builder::{build_assistant_message, finalize_explore_only_turn};
use self::persistence::{
    clear_recovery_state, persist_context_state, persist_recovery_state, save_message,
    set_agent_state, update_token_count,
};
use self::plan_flow::{handle_plan_detection, PlanDetectionOutcome};
use self::recovery::{
    build_awaiting_input_recovery_state, build_partial_assistant_state, build_recovery_state,
    continuation_recovery_message,
};
use self::title::maybe_generate_title;

const EXPLORATION_BUDGET_SOFT: usize = 15;
const EXPLORATION_BUDGET_HARD: usize = 30;
const EMPTY_COMPLETION_RECOVERY_INSTRUCTION: &str = "[EMPTY RESPONSE RECOVERY]\nThe previous model completion contained no user-visible text or tool call. Continue the same turn from the existing conversation and provide the response requested by the user now, or make a necessary new tool call. Do not repeat a completed tool call merely because the prior completion was empty, and do not mention this recovery instruction.";
const EMPTY_COMPLETION_ERROR: &str = "The AI provider completed twice without producing user-visible text or a tool call. Try again or choose another model.";
const EMPTY_COMPLETION_AFTER_SERVER_TOOL_ERROR: &str = "The AI provider completed after hosted tool activity without producing a user-visible response. The hosted tool was not replayed; try again or choose another model.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmptyCompletionAction {
    None,
    Retry,
    Fail,
}

fn empty_completion_action(
    text: &str,
    tool_calls: &[AiToolCall],
    retry_attempted: bool,
    had_server_tool_activity: bool,
) -> EmptyCompletionAction {
    if !text.trim().is_empty() || !tool_calls.is_empty() {
        EmptyCompletionAction::None
    } else if retry_attempted || had_server_tool_activity {
        EmptyCompletionAction::Fail
    } else {
        EmptyCompletionAction::Retry
    }
}

fn terminal_agent_state_after_interruption(stop_reason: &LoopStopReason) -> &'static str {
    match stop_reason {
        LoopStopReason::StreamIdleTimeout | LoopStopReason::UserAbort => "idle",
        LoopStopReason::ProviderError | LoopStopReason::PinchFailed => "error",
        _ => "idle",
    }
}

fn should_retry_empty_stream_interruption(
    stop_reason: Option<&LoopStopReason>,
    last_error: Option<&str>,
    produced_output: bool,
    retry_attempted: bool,
) -> bool {
    if retry_attempted || produced_output {
        return false;
    }

    match stop_reason {
        Some(LoopStopReason::StreamIdleTimeout) => true,
        Some(LoopStopReason::ProviderError) => last_error.is_some_and(is_retryable_error_message),
        _ => false,
    }
}

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

fn resolve_project_permission_mode(
    requested_permission_mode: PermissionMode,
    project_settings: &ProjectSettings,
) -> PermissionMode {
    let Some(ref mode_str) = project_settings.permission_mode else {
        return requested_permission_mode;
    };

    match mode_str.as_str() {
        "supervised" => {
            tracing::info!("Project settings override: permission_mode = supervised");
            PermissionMode::Supervised
        }
        "autonomous" if requested_permission_mode == PermissionMode::Autonomous => {
            tracing::info!("Project settings confirm: permission_mode = autonomous");
            PermissionMode::Autonomous
        }
        "autonomous" => {
            tracing::warn!(
                "Ignoring project settings permission_mode = autonomous because the request is supervised"
            );
            requested_permission_mode
        }
        other => {
            tracing::warn!(
                "Unknown permission_mode in project settings: {:?}, keeping requested mode",
                other
            );
            requested_permission_mode
        }
    }
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
        let (provider_call_tx, provider_call_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        let trace_db_path = self.services.db_path.clone();
        let trace_session_id = self.config.session_id.clone();
        let trace_run_id = super::observability::new_runtime_trace_run_id();
        let extension_dispatch =
            self.services
                .tool_registry
                .agent_extension_manager()
                .map(|manager| {
                    let context = crate::extensions::ExtensionCallContext::for_turn(
                        self.config.working_dir.clone(),
                        self.config.project_dir.clone(),
                        Some(self.config.session_id.clone()),
                        Some(self.services.ai_client.config().model.clone()),
                        format!("{:?}", self.config.permission_mode).to_ascii_lowercase(),
                        matches!(self.config.initial_work_mode, WorkMode::Plan),
                    );
                    (manager, context)
                });

        tokio::spawn(async move {
            super::observability::forward_runtime_traces(
                trace_db_path,
                trace_session_id,
                trace_run_id,
                trace_rx,
                provider_call_rx,
                event_tx,
                extension_dispatch,
            )
            .await;
        });

        tokio::spawn(async move {
            self.run_inner(conversation, options, trace_tx, provider_call_tx, input_rx)
                .await;
        });

        (event_rx, input_tx)
    }

    async fn run_inner(
        self,
        mut conversation: Vec<ModelMessage>,
        options: CallOptions,
        event_tx: mpsc::UnboundedSender<LoopEvent>,
        provider_call_tx: mpsc::UnboundedSender<super::observability::ProviderCallTrace>,
        input_rx: mpsc::UnboundedReceiver<LoopInput>,
    ) {
        let mut input_inbox = LoopInputInbox::new(input_rx);
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

        let permission_mode = resolve_project_permission_mode(permission_mode, &project_settings);

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
        let mut last_usage_prompt_tokens = None::<usize>;
        let mut messages_at_last_usage = 0usize;
        let mut exploration_budget_count = 0usize;
        let mut tool_failure_signatures: HashMap<String, usize> = HashMap::new();
        let mut tool_pattern_signatures: HashMap<String, usize> = HashMap::new();
        let mut validation_pattern_signatures: HashMap<String, usize> = HashMap::new();
        let mut title_generated = !generate_title;
        let mut iteration = 0usize;
        let model_context_window = effective_context_window_for_runtime(
            ai_client.config().uses_chatgpt_codex_format(),
            resolve_context_window(
                ai_client.provider_id(),
                &ai_client.config().model,
                ai_client.config().api_format,
            ),
        );
        let compaction_manager = CompactionManager::for_model(
            ai_client.provider_id(),
            ai_client.config().api_format,
            &ai_client.config().model,
            model_context_window,
        );
        let mut context_ledger = ContextLedger::from_conversation(&conversation);
        let mut empty_stream_retry_attempted = false;
        let mut empty_completion_retry_attempted = false;
        let mut empty_completion_recovery_pending = false;
        let mut provider_tool_activity_seen = false;
        let mut overflow_compact_retry_attempted = false;
        let mut mutation_needs_validation = false;
        let project_dir_key = project_dir
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned());
        let project_dir_for_compaction = project_dir_key.as_deref();
        let user_id_for_compaction = user_id.as_deref();
        let file_observations = Arc::new(FileObservationTracker::default());
        persist_context_state(&db_path, &session_id, &context_ledger);
        clear_recovery_state(&db_path, &session_id);

        let model_profile = ModelProfile::resolve(
            ai_client.provider_id(),
            ai_client.config().api_format,
            &ai_client.config().model,
        );
        let effective_stream_idle_timeout = stream_idle_timeout.max(
            std::time::Duration::from_secs(model_profile.stream_idle_timeout_secs),
        );

        set_agent_state(&db_path, &session_id, "streaming");

        loop {
            input_inbox.collect_ready();
            if input_inbox.take_cancel() {
                clear_recovery_state(&db_path, &session_id);
                set_agent_state(&db_path, &session_id, "idle");
                let _ = event_tx.send(LoopEvent::Finished {
                    session_id: session_id.clone(),
                    stop_reason: LoopStopReason::UserAbort,
                });
                return;
            }
            let injected_steering = inject_pending_steering(
                &mut input_inbox,
                &mut conversation,
                &mut context_ledger,
                &db_path,
                &session_id,
            );
            if !injected_steering.is_empty() {
                emit_steering_events(&event_tx, injected_steering);
                empty_stream_retry_attempted = false;
                empty_completion_retry_attempted = false;
                empty_completion_recovery_pending = false;
                provider_tool_activity_seen = false;
                overflow_compact_retry_attempted = false;
                exploration_budget_count = 0;
                tool_failure_signatures.clear();
                tool_pattern_signatures.clear();
                validation_pattern_signatures.clear();
            }

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
            let provider_call_trace = super::observability::ProviderCallTraceContext::for_run(
                provider_call_tx.clone(),
                iteration,
            );

            let micro = microcompact_messages(&conversation);
            if micro.changed {
                conversation = micro.messages;
                context_ledger.update_from_conversation(&conversation);
                persist_context_state(&db_path, &session_id, &context_ledger);
            }

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
                user_id.as_deref(),
            );
            if let Some(extension_manager) = tool_registry.agent_extension_manager() {
                let extension_context = crate::extensions::ExtensionCallContext::for_turn(
                    working_dir.clone(),
                    project_dir.clone(),
                    Some(session_id.clone()),
                    Some(ai_client.config().model.clone()),
                    format!("{:?}", permission_mode).to_ascii_lowercase(),
                    matches!(work_mode, WorkMode::Plan),
                );
                let extension_context_additions =
                    extension_manager.context_for_turn(&extension_context).await;
                if !extension_context_additions.is_empty() {
                    conversation_with_context.push(ModelMessage {
                        role: Role::System,
                        content: vec![Content::Text {
                            text: format!(
                                "[AGENT EXTENSION CONTEXT]\n\n{}\n\n[END AGENT EXTENSION CONTEXT]",
                                extension_context_additions.join("\n\n")
                            ),
                        }],
                    });
                }
            }
            if empty_completion_recovery_pending {
                conversation_with_context.push(ModelMessage {
                    role: Role::System,
                    content: vec![Content::Text {
                        text: EMPTY_COMPLETION_RECOVERY_INSTRUCTION.to_string(),
                    }],
                });
            }
            let request_estimate = super::estimate_rendered_request_tokens(
                ai_client.as_ref(),
                &conversation_with_context,
                &options,
            );
            let usage_calibrated_estimate = super::estimate_tokens_with_usage(
                &conversation,
                last_usage_prompt_tokens,
                conversation.len().saturating_sub(messages_at_last_usage),
            );
            let estimated_tokens_before =
                request_estimate.total_tokens.max(usage_calibrated_estimate);
            let compaction_request_budget =
                request_estimate.compaction_budget(estimated_tokens_before);
            tracing::debug!(
                session_id = %session_id,
                base_prompt_tokens = request_estimate.base_prompt_tokens,
                project_context_tokens = request_estimate.project_context_tokens,
                session_context_tokens = request_estimate.session_context_tokens,
                message_tokens = request_estimate.message_tokens,
                tool_tokens = request_estimate.tool_tokens,
                usage_calibrated_tokens = usage_calibrated_estimate,
                total_tokens = request_estimate.total_tokens,
                compaction_pressure_tokens = estimated_tokens_before,
                "Preflight rendered-request token estimate"
            );

            if compaction_manager.should_compact(estimated_tokens_before) {
                let messages_after_usage =
                    conversation.len().saturating_sub(messages_at_last_usage);
                match apply_in_place_compaction(
                    &db_path,
                    &session_id,
                    &mut conversation,
                    &mut context_ledger,
                    &working_dir,
                    project_dir_for_compaction,
                    user_id_for_compaction,
                    ai_client.as_ref(),
                    &compaction_manager,
                    CompactionTrigger::Auto,
                    last_usage_prompt_tokens,
                    messages_after_usage,
                    Some(compaction_request_budget),
                    &provider_call_trace,
                    &event_tx,
                )
                .await
                {
                    Ok(result) => {
                        last_usage_prompt_tokens = None;
                        messages_at_last_usage = conversation.len();
                        last_token_count = result.estimated_tokens_after;
                        update_token_count(&db_path, &session_id, result.estimated_tokens_after);
                        clear_recovery_state(&db_path, &session_id);
                        set_agent_state(&db_path, &session_id, "streaming");
                        continue;
                    }
                    Err(error) => {
                        let _ = event_tx.send(LoopEvent::Error {
                            error: format!("Automatic compaction failed: {}", error),
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
            let provider_call_id = uuid::Uuid::new_v4().to_string();
            let provider_call_started = Instant::now();
            let streaming_setup = ai_client.call_streaming(conversation_with_context, &options);
            tokio::pin!(streaming_setup);
            let mut setup_input_closed = false;
            let setup_result = loop {
                tokio::select! {
                    result = &mut streaming_setup => break Some(result),
                    cancelled = input_inbox.recv_cancel(), if !setup_input_closed => {
                        match cancelled {
                            Some(()) => break None,
                            None => setup_input_closed = true,
                        }
                    }
                }
            };

            let Some(setup_result) = setup_result else {
                let _ = provider_call_tx.send(super::observability::ProviderCallTrace::agent_loop(
                    provider_call_id,
                    iteration,
                    ai_client.provider_id(),
                    &ai_client.config().model,
                    "cancelled_during_setup",
                    None,
                    provider_call_started.elapsed(),
                ));
                clear_recovery_state(&db_path, &session_id);
                set_agent_state(&db_path, &session_id, "idle");
                let _ = event_tx.send(LoopEvent::Finished {
                    session_id: session_id.clone(),
                    stop_reason: LoopStopReason::UserAbort,
                });
                return;
            };

            let api_rx = match setup_result {
                Ok(rx) => rx,
                Err(e) => {
                    let _ =
                        provider_call_tx.send(super::observability::ProviderCallTrace::agent_loop(
                            provider_call_id,
                            iteration,
                            ai_client.provider_id(),
                            &ai_client.config().model,
                            "setup_error",
                            None,
                            provider_call_started.elapsed(),
                        ));
                    let error = format!("AI error: {}", e);
                    if !overflow_compact_retry_attempted && is_context_overflow_error(&error) {
                        overflow_compact_retry_attempted = true;
                        tracing::warn!(
                            session_id = %session_id,
                            "Provider rejected request for context overflow; compacting and retrying once"
                        );
                        let messages_after_usage =
                            conversation.len().saturating_sub(messages_at_last_usage);
                        match apply_in_place_compaction(
                            &db_path,
                            &session_id,
                            &mut conversation,
                            &mut context_ledger,
                            &working_dir,
                            project_dir_for_compaction,
                            user_id_for_compaction,
                            ai_client.as_ref(),
                            &compaction_manager,
                            CompactionTrigger::Overflow,
                            last_usage_prompt_tokens,
                            messages_after_usage,
                            Some(compaction_request_budget),
                            &provider_call_trace,
                            &event_tx,
                        )
                        .await
                        {
                            Ok(result) => {
                                last_usage_prompt_tokens = None;
                                messages_at_last_usage = conversation.len();
                                last_token_count = result.estimated_tokens_after;
                                update_token_count(
                                    &db_path,
                                    &session_id,
                                    result.estimated_tokens_after,
                                );
                                clear_recovery_state(&db_path, &session_id);
                                set_agent_state(&db_path, &session_id, "streaming");
                                continue;
                            }
                            Err(compaction_error) => {
                                tracing::error!(
                                    session_id = %session_id,
                                    error = %compaction_error,
                                    "Reactive overflow compaction failed"
                                );
                            }
                        }
                    }
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

            let result = {
                let stream_processing = stream::process_stream(
                    api_rx,
                    &event_tx,
                    effective_stream_idle_timeout,
                    |checkpoint| {
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
                    },
                );
                tokio::pin!(stream_processing);
                let mut input_closed = false;
                loop {
                    tokio::select! {
                        result = &mut stream_processing => break Some(result),
                        cancelled = input_inbox.recv_cancel(), if !input_closed => {
                            match cancelled {
                                Some(()) => break None,
                                None => input_closed = true,
                            }
                        }
                    }
                }
            };

            let Some(result) = result else {
                let _ = provider_call_tx.send(super::observability::ProviderCallTrace::agent_loop(
                    provider_call_id,
                    iteration,
                    ai_client.provider_id(),
                    &ai_client.config().model,
                    "cancelled_during_stream",
                    None,
                    provider_call_started.elapsed(),
                ));
                clear_recovery_state(&db_path, &session_id);
                set_agent_state(&db_path, &session_id, "idle");
                let _ = event_tx.send(LoopEvent::Finished {
                    session_id: session_id.clone(),
                    stop_reason: LoopStopReason::UserAbort,
                });
                return;
            };

            let provider_call_outcome = match result.stop_reason.as_ref() {
                None => "completed",
                Some(LoopStopReason::ProviderError) => "provider_error",
                Some(LoopStopReason::StreamIdleTimeout) => "stream_idle_timeout",
                Some(LoopStopReason::UserAbort) => "user_abort",
                Some(_) => "interrupted",
            };
            let _ = provider_call_tx.send(super::observability::ProviderCallTrace::agent_loop(
                provider_call_id,
                iteration,
                ai_client.provider_id(),
                &ai_client.config().model,
                provider_call_outcome,
                result.usage_available.then_some(result.usage.clone()),
                provider_call_started.elapsed(),
            ));

            if result.total_tokens > 0 {
                last_token_count = result.total_tokens;
            }
            if result.prompt_tokens > 0 {
                last_usage_prompt_tokens = Some(result.prompt_tokens);
                messages_at_last_usage = conversation.len();
            }
            provider_tool_activity_seen |= result.had_server_tool_activity;

            if should_retry_empty_stream_interruption(
                result.stop_reason.as_ref(),
                result.last_error.as_deref(),
                result.produced_output,
                empty_stream_retry_attempted,
            ) {
                empty_stream_retry_attempted = true;
                tracing::warn!(
                    session_id = %session_id,
                    "Provider stream ended before text or tool calls; retrying once"
                );
                clear_recovery_state(&db_path, &session_id);
                set_agent_state(&db_path, &session_id, "streaming");
                continue;
            }

            if let Some(stop_reason) = result.stop_reason.clone() {
                if !overflow_compact_retry_attempted
                    && stop_reason == LoopStopReason::ProviderError
                    && result
                        .last_error
                        .as_ref()
                        .is_some_and(|error| is_context_overflow_error(error))
                {
                    overflow_compact_retry_attempted = true;
                    tracing::warn!(
                        session_id = %session_id,
                        "Provider stream reported context overflow; compacting and retrying once"
                    );
                    let messages_after_usage =
                        conversation.len().saturating_sub(messages_at_last_usage);
                    match apply_in_place_compaction(
                        &db_path,
                        &session_id,
                        &mut conversation,
                        &mut context_ledger,
                        &working_dir,
                        project_dir_for_compaction,
                        user_id_for_compaction,
                        ai_client.as_ref(),
                        &compaction_manager,
                        CompactionTrigger::Overflow,
                        last_usage_prompt_tokens,
                        messages_after_usage,
                        Some(compaction_request_budget),
                        &provider_call_trace,
                        &event_tx,
                    )
                    .await
                    {
                        Ok(result) => {
                            last_usage_prompt_tokens = None;
                            messages_at_last_usage = conversation.len();
                            last_token_count = result.estimated_tokens_after;
                            update_token_count(
                                &db_path,
                                &session_id,
                                result.estimated_tokens_after,
                            );
                            clear_recovery_state(&db_path, &session_id);
                            set_agent_state(&db_path, &session_id, "streaming");
                            continue;
                        }
                        Err(compaction_error) => {
                            tracing::error!(
                                session_id = %session_id,
                                error = %compaction_error,
                                "Reactive overflow compaction failed"
                            );
                        }
                    }
                }
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
                let terminal_error = result
                    .last_error
                    .clone()
                    .unwrap_or_else(|| continuation_recovery_message(&context_ledger));
                let _ = event_tx.send(LoopEvent::Error {
                    error: terminal_error,
                });
                if last_token_count > 0 {
                    update_token_count(&db_path, &session_id, last_token_count);
                }
                set_agent_state(
                    &db_path,
                    &session_id,
                    terminal_agent_state_after_interruption(&stop_reason),
                );
                let _ = event_tx.send(LoopEvent::Finished {
                    session_id: session_id.clone(),
                    stop_reason,
                });
                return;
            }

            // A provider can successfully finish after emitting only internal
            // reasoning. That is a valid transport response but not a usable
            // assistant turn. Recover semantically once without replaying any
            // completed tool call or polluting canonical conversation history.
            empty_completion_recovery_pending = false;
            let empty_completion = if session_type == SessionType::Mako {
                EmptyCompletionAction::None
            } else {
                empty_completion_action(
                    &result.text,
                    &result.tool_calls,
                    empty_completion_retry_attempted,
                    provider_tool_activity_seen,
                )
            };
            match empty_completion {
                EmptyCompletionAction::Retry => {
                    empty_completion_retry_attempted = true;
                    empty_completion_recovery_pending = true;
                    tracing::warn!(
                        session_id = %session_id,
                        "Provider completed without user-visible text or a tool call; requesting one semantic continuation"
                    );
                    clear_recovery_state(&db_path, &session_id);
                    set_agent_state(&db_path, &session_id, "streaming");
                    continue;
                }
                EmptyCompletionAction::Fail => {
                    let error = if provider_tool_activity_seen {
                        EMPTY_COMPLETION_AFTER_SERVER_TOOL_ERROR
                    } else {
                        EMPTY_COMPLETION_ERROR
                    };
                    tracing::error!(
                        session_id = %session_id,
                        had_server_tool_activity = provider_tool_activity_seen,
                        "Provider produced an unrecoverable empty semantic completion"
                    );
                    persist_recovery_state(
                        &db_path,
                        &session_id,
                        &build_recovery_state(
                            &context_ledger,
                            RecoveryStatus::Interrupted,
                            Some(LoopStopReason::ProviderError),
                            Some(error.to_string()),
                            build_partial_assistant_state(&result.recovery_checkpoint),
                        ),
                    );
                    persist_context_state(&db_path, &session_id, &context_ledger);
                    let _ = event_tx.send(LoopEvent::Error {
                        error: error.to_string(),
                    });
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
                EmptyCompletionAction::None => {}
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

            input_inbox.collect_ready();
            if input_inbox.take_cancel() {
                clear_recovery_state(&db_path, &session_id);
                set_agent_state(&db_path, &session_id, "idle");
                let _ = event_tx.send(LoopEvent::Finished {
                    session_id: session_id.clone(),
                    stop_reason: LoopStopReason::UserAbort,
                });
                return;
            }

            // Title generation on first response
            if !title_generated && !result.text.is_empty() {
                title_generated = true;
                maybe_generate_title(&conversation, &event_tx, &session_id, &db_path);
            }

            // A tool-free completion is also a safe boundary for live steering.
            if result.tool_calls.is_empty() {
                let injected_steering = inject_pending_steering(
                    &mut input_inbox,
                    &mut conversation,
                    &mut context_ledger,
                    &db_path,
                    &session_id,
                );
                if no_tool_completion_should_continue(&injected_steering) {
                    empty_stream_retry_attempted = false;
                    empty_completion_retry_attempted = false;
                    empty_completion_recovery_pending = false;
                    provider_tool_activity_seen = false;
                    overflow_compact_retry_attempted = false;
                    exploration_budget_count = 0;
                    tool_failure_signatures.clear();
                    tool_pattern_signatures.clear();
                    validation_pattern_signatures.clear();
                    clear_recovery_state(&db_path, &session_id);
                    set_agent_state(&db_path, &session_id, "streaming");
                    let _ = event_tx.send(LoopEvent::TurnComplete {
                        turn: iteration,
                        has_more: true,
                    });
                    emit_steering_events(&event_tx, injected_steering);
                    continue;
                }
            }

            // Supervised planning retains the explicit confirmation boundary.
            // Autonomous planning promotes a detected plan to Build mode at the
            // same canonical boundary. Detect before tool execution in that mode
            // so a provider that returns a plan and its first write together does
            // not bounce against the read-only PlanModeHook first.
            let should_detect_plan = should_detect_plan_transition(
                work_mode,
                permission_mode,
                !result.tool_calls.is_empty(),
            );
            if should_detect_plan {
                if let Some(outcome) = handle_plan_detection(
                    &result.text,
                    &session_id,
                    &working_dir,
                    &db_path,
                    permission_mode,
                    &event_tx,
                ) {
                    match outcome {
                        PlanDetectionOutcome::AwaitingConfirmation(pending_interaction) => {
                            // The server's tool-result handler manages supervised confirmation.
                            if last_token_count > 0 {
                                update_token_count(&db_path, &session_id, last_token_count);
                            }
                            persist_recovery_state(
                                &db_path,
                                &session_id,
                                &build_awaiting_input_recovery_state(
                                    build_partial_assistant_state(&result.recovery_checkpoint),
                                    vec![pending_interaction],
                                    permission_mode,
                                ),
                            );
                            set_agent_state(&db_path, &session_id, "awaiting_input");
                            let _ = event_tx.send(LoopEvent::Finished {
                                session_id: session_id.clone(),
                                stop_reason: LoopStopReason::AwaitingInput,
                            });
                            return;
                        }
                        PlanDetectionOutcome::ContinueInBuildMode => {
                            work_mode = WorkMode::Build;
                            if result.tool_calls.is_empty() {
                                if last_token_count > 0 {
                                    update_token_count(&db_path, &session_id, last_token_count);
                                }
                                clear_recovery_state(&db_path, &session_id);
                                set_agent_state(&db_path, &session_id, "streaming");
                                let _ = event_tx.send(LoopEvent::TurnComplete {
                                    turn: iteration,
                                    has_more: true,
                                });
                                continue;
                            }
                        }
                        PlanDetectionOutcome::Failed(error) => {
                            tracing::error!(
                                session_id = %session_id,
                                %error,
                                "Plan lifecycle transition failed"
                            );
                            if last_token_count > 0 {
                                update_token_count(&db_path, &session_id, last_token_count);
                            }
                            persist_recovery_state(
                                &db_path,
                                &session_id,
                                &build_recovery_state(
                                    &context_ledger,
                                    RecoveryStatus::Interrupted,
                                    Some(LoopStopReason::ProviderError),
                                    Some(error.clone()),
                                    build_partial_assistant_state(&result.recovery_checkpoint),
                                ),
                            );
                            set_agent_state(&db_path, &session_id, "error");
                            let _ = event_tx.send(LoopEvent::Error { error });
                            let _ = event_tx.send(LoopEvent::Finished {
                                session_id: session_id.clone(),
                                stop_reason: LoopStopReason::ProviderError,
                            });
                            return;
                        }
                    }
                }
            }

            // No tool calls and no plan transition → finish turn.
            if result.tool_calls.is_empty() {
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
                    let other_batch = executor::execute_tools(
                        &other_calls,
                        &tool_registry,
                        &ai_client,
                        &working_dir,
                        project_dir.as_deref(),
                        &process_registry,
                        &skills_manager,
                        &session_id,
                        &db_path,
                        user_id.as_deref(),
                        permission_mode,
                        work_mode,
                        Some(&ask_user_partial_assistant),
                        delegated_progress_tx.as_ref(),
                        &event_tx,
                        Some(&provider_call_trace),
                        &mut input_inbox,
                        project_settings.subagent_max_turns,
                        project_settings.disabled_tools.as_deref(),
                        Arc::clone(&file_observations),
                    )
                    .await;
                    all_results.extend(other_batch.results);
                    if other_batch.cancelled {
                        clear_recovery_state(&db_path, &session_id);
                        set_agent_state(&db_path, &session_id, "idle");
                        let _ = event_tx.send(LoopEvent::Finished {
                            session_id: session_id.clone(),
                            stop_reason: LoopStopReason::UserAbort,
                        });
                        return;
                    }
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

                input_inbox.collect_ready();
                if input_inbox.take_cancel() {
                    clear_recovery_state(&db_path, &session_id);
                    set_agent_state(&db_path, &session_id, "idle");
                    let _ = event_tx.send(LoopEvent::Finished {
                        session_id: session_id.clone(),
                        stop_reason: LoopStopReason::UserAbort,
                    });
                    return;
                }
                let injected_steering = inject_pending_steering(
                    &mut input_inbox,
                    &mut conversation,
                    &mut context_ledger,
                    &db_path,
                    &session_id,
                );
                emit_steering_events(&event_tx, injected_steering);

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
                        permission_mode,
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
            let tool_batch = executor::execute_tools(
                &result.tool_calls,
                &tool_registry,
                &ai_client,
                &working_dir,
                project_dir.as_deref(),
                &process_registry,
                &skills_manager,
                &session_id,
                &db_path,
                user_id.as_deref(),
                permission_mode,
                work_mode,
                Some(&tool_execution_partial_assistant),
                delegated_progress_tx.as_ref(),
                &event_tx,
                Some(&provider_call_trace),
                &mut input_inbox,
                project_settings.subagent_max_turns,
                project_settings.disabled_tools.as_deref(),
                Arc::clone(&file_observations),
            )
            .await;
            work_mode = tool_batch.next_work_mode;
            let tool_results = tool_batch.results;

            if tool_batch.cancelled {
                let tool_msg = ModelMessage {
                    role: Role::User,
                    content: tool_results,
                };
                save_message(&db_path, &session_id, &tool_msg);
                clear_recovery_state(&db_path, &session_id);
                set_agent_state(&db_path, &session_id, "idle");
                let _ = event_tx.send(LoopEvent::Finished {
                    session_id: session_id.clone(),
                    stop_reason: LoopStopReason::UserAbort,
                });
                return;
            }

            // Failure detection
            let fail_diagnostic = failure::detect_repeated_failures(
                &mut tool_failure_signatures,
                &result.tool_calls,
                &tool_results,
            );
            let explore_diagnostic =
                failure::detect_terminal_explore_failure(&result.tool_calls, &tool_results);
            let validation_diagnostic = failure::detect_repeated_validation_sequence(
                &mut validation_pattern_signatures,
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
            let validation_reminder_needed = update_validation_state(
                &mut mutation_needs_validation,
                &result.tool_calls,
                &tool_msg.content,
            );
            conversation.push(tool_msg.clone());
            if !mutation_needs_validation {
                remove_validation_reminders(&mut conversation);
            }
            if validation_reminder_needed && session_type == SessionType::Code {
                conversation.push(ModelMessage {
                    role: Role::System,
                    content: vec![Content::Text {
                        text: VALIDATION_REMINDER.to_string(),
                    }],
                });
            }
            context_ledger.update_from_conversation(&conversation);
            persist_context_state(&db_path, &session_id, &context_ledger);
            save_message(&db_path, &session_id, &tool_msg);
            clear_recovery_state(&db_path, &session_id);

            input_inbox.collect_ready();
            if input_inbox.take_cancel() {
                set_agent_state(&db_path, &session_id, "idle");
                let _ = event_tx.send(LoopEvent::Finished {
                    session_id: session_id.clone(),
                    stop_reason: LoopStopReason::UserAbort,
                });
                return;
            }
            let injected_steering = inject_pending_steering(
                &mut input_inbox,
                &mut conversation,
                &mut context_ledger,
                &db_path,
                &session_id,
            );
            if !injected_steering.is_empty() {
                empty_stream_retry_attempted = false;
                empty_completion_retry_attempted = false;
                empty_completion_recovery_pending = false;
                provider_tool_activity_seen = false;
                overflow_compact_retry_attempted = false;
                exploration_budget_count = 0;
                tool_failure_signatures.clear();
                tool_pattern_signatures.clear();
                validation_pattern_signatures.clear();
                set_agent_state(&db_path, &session_id, "streaming");
                let _ = event_tx.send(LoopEvent::TurnComplete {
                    turn: iteration,
                    has_more: true,
                });
                emit_steering_events(&event_tx, injected_steering);
                continue;
            }

            // Check fail-fast
            if let Some(diagnostic) = fail_diagnostic
                .or(explore_diagnostic)
                .or(validation_diagnostic)
            {
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
                    maybe_generate_title(&conversation, &event_tx, &session_id, &db_path);
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

/// Promote each queued user follow-up at a model boundary and append it to the
/// in-memory conversation exactly once. Server-originated inputs are first
/// staged under a non-canonical role; promotion deletes that staging row and
/// appends the canonical user message in one SQLite transaction.
struct InjectedSteering {
    pending_id: Option<String>,
    message: String,
}

fn no_tool_completion_should_continue(steering: &[InjectedSteering]) -> bool {
    !steering.is_empty()
}

fn should_detect_plan_transition(
    work_mode: WorkMode,
    permission_mode: PermissionMode,
    has_tool_calls: bool,
) -> bool {
    work_mode == WorkMode::Plan
        && (!has_tool_calls || permission_mode == PermissionMode::Autonomous)
}

fn emit_steering_events(
    event_tx: &mpsc::UnboundedSender<LoopEvent>,
    steering: Vec<InjectedSteering>,
) {
    for input in steering {
        let _ = event_tx.send(LoopEvent::SteeringInjected {
            pending_id: input.pending_id,
            message: input.message,
        });
    }
}

fn steering_display_message(content: &[Content]) -> String {
    let text = content
        .iter()
        .filter_map(|item| match item {
            Content::Text { text } if !text.trim().is_empty() => Some(text.trim()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    if text.is_empty() {
        "[Attached content]".to_string()
    } else {
        text
    }
}

fn inject_pending_steering(
    input_inbox: &mut LoopInputInbox,
    conversation: &mut Vec<ModelMessage>,
    context_ledger: &mut ContextLedger,
    db_path: &Path,
    session_id: &str,
) -> Vec<InjectedSteering> {
    let pending = input_inbox.take_steering();
    if pending.is_empty() {
        return Vec::new();
    }

    let session_manager = Database::new(db_path).ok().map(SessionManager::new);
    let mut injected = Vec::new();

    for steering in pending {
        if steering.content.is_empty() {
            continue;
        }

        if let Some(pending_id) = steering.pending_id.as_deref() {
            let promoted = session_manager
                .as_ref()
                .and_then(|manager| {
                    manager
                        .promote_pending_steering(session_id, pending_id)
                        .map_err(|error| {
                            tracing::warn!(
                                session_id,
                                pending_id,
                                %error,
                                "Failed to promote durable live steering input"
                            );
                            error
                        })
                        .ok()
                })
                .flatten();
            if promoted.is_none() {
                // Missing staging rows are duplicate deliveries; failed
                // promotions remain durable for restart recovery.
                continue;
            }
        }

        let display_message = steering_display_message(&steering.content);
        let message = ModelMessage {
            role: Role::User,
            content: steering.content,
        };
        conversation.push(message.clone());
        if steering.pending_id.is_none() {
            save_message(db_path, session_id, &message);
        }
        injected.push(InjectedSteering {
            pending_id: steering.pending_id,
            message: display_message,
        });
    }

    if !injected.is_empty() {
        context_ledger.update_from_conversation(conversation);
        persist_context_state(db_path, session_id, context_ledger);
    }
    injected
}

fn update_validation_state(
    mutation_needs_validation: &mut bool,
    tool_calls: &[crate::ai::types::AiToolCall],
    tool_results: &[Content],
) -> bool {
    let successful_ids = tool_results
        .iter()
        .filter_map(|result| match result {
            Content::ToolResult {
                tool_use_id,
                is_error,
                ..
            } if !is_error.unwrap_or(false) => Some(tool_use_id.as_str()),
            _ => None,
        })
        .collect::<std::collections::HashSet<_>>();
    let was_pending = *mutation_needs_validation;

    for call in tool_calls
        .iter()
        .filter(|call| successful_ids.contains(call.id.as_str()))
    {
        if failure::is_validation_call(call) {
            *mutation_needs_validation = false;
        } else if is_mutation_call(call) {
            *mutation_needs_validation = true;
        }
    }

    !was_pending && *mutation_needs_validation
}

const VALIDATION_REMINDER: &str = "Files changed successfully. Before finishing, run the narrowest relevant test, build, lint, typecheck, or `git diff --check`. If no executable validation applies, say why explicitly instead of implying it ran.";

fn remove_validation_reminders(conversation: &mut Vec<ModelMessage>) {
    conversation.retain(|message| {
        !(message.role == Role::System
            && message.content.iter().any(
                |content| matches!(content, Content::Text { text } if text == VALIDATION_REMINDER),
            ))
    });
}

fn is_mutation_call(call: &crate::ai::types::AiToolCall) -> bool {
    let (name, arguments) = effective_tool_call(&call.name, &call.arguments);
    match name {
        "edit" | "write" | "multiedit" | "apply_patch" => true,
        "agent" => arguments.get("agent_type").and_then(|value| value.as_str()) == Some("build"),
        "bash" | "shell" | "execute" => arguments
            .get("command")
            .and_then(|value| value.as_str())
            .is_some_and(|command| {
                super::hooks::shell_policy::classify_bash_command(command)
                    .modifies_filesystem_or_process
            }),
        _ => false,
    }
}

async fn apply_in_place_compaction(
    db_path: &Path,
    session_id: &str,
    conversation: &mut Vec<ModelMessage>,
    context_ledger: &mut ContextLedger,
    working_dir: &Path,
    project_dir: Option<&str>,
    user_id: Option<&str>,
    ai_client: &AiClient,
    compaction_manager: &CompactionManager,
    trigger: CompactionTrigger,
    last_usage_prompt_tokens: Option<usize>,
    messages_after_usage: usize,
    request_budget: Option<CompactionRequestBudget>,
    provider_call_trace: &super::ProviderCallTraceContext,
    event_tx: &mpsc::UnboundedSender<LoopEvent>,
) -> Result<CompactionResult, anyhow::Error> {
    let reason = match &trigger {
        CompactionTrigger::Auto => "auto",
        CompactionTrigger::Manual { .. } => "manual",
        CompactionTrigger::Reactive => "reactive",
        CompactionTrigger::Overflow => "overflow",
    }
    .to_string();

    let _ = event_tx.send(LoopEvent::ContextCompactionStarted {
        reason: reason.clone(),
    });

    let compaction_result = run_compaction_pipeline_observed(
        CompactionRequest {
            db_path,
            session_id,
            conversation,
            working_dir,
            ai_client: Some(ai_client),
            model: Some(ai_client.config().model.as_str()),
            trigger,
            compaction_manager: *compaction_manager,
            request_budget,
            last_usage_prompt_tokens,
            messages_after_usage,
            summary_override: None,
            project_dir,
            user_id,
        },
        provider_call_trace,
    )
    .await?;

    *conversation = compaction_result.compacted_conversation.clone();
    context_ledger.update_from_conversation(conversation);
    persist_context_state(db_path, session_id, context_ledger);
    let _ = event_tx.send(LoopEvent::ContextCompacted {
        reason: reason.clone(),
        estimated_tokens_before: compaction_result.estimated_tokens_before,
        estimated_tokens_after: compaction_result.estimated_tokens_after,
        replaced_messages: compaction_result.replaced_messages,
        checkpoint_id: compaction_result.checkpoint_id.clone(),
        compaction_count: compaction_result.compaction_count,
    });
    tracing::info!(
        session_id = %session_id,
        reason = %reason,
        tokens_before = compaction_result.estimated_tokens_before,
        tokens_after = compaction_result.estimated_tokens_after,
        replaced = compaction_result.replaced_messages,
        "In-place compaction completed"
    );

    Ok(compaction_result)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::effective_context_window_for_runtime;
    use super::empty_completion_action;
    use super::inject_pending_steering;
    use super::inject_runtime_context;
    use super::message_builder::finalize_explore_only_turn;
    use super::no_tool_completion_should_continue;
    use super::remove_validation_reminders;
    use super::resolve_project_permission_mode;
    use super::should_detect_plan_transition;
    use super::should_retry_empty_stream_interruption;
    use super::terminal_agent_state_after_interruption;
    use super::update_validation_state;
    use super::EmptyCompletionAction;
    use super::VALIDATION_REMINDER;
    use super::{mpsc, ContextLedger};
    use crate::agent::loop_events::{LoopInput, LoopInputInbox, LoopStopReason};
    use crate::ai::types::{AiToolCall, Content, ModelMessage, Role};
    use crate::skills::SkillsManager;
    use crate::storage::{Database, ProjectSettings, SessionManager, SessionType, WorkMode};
    use crate::tools::registry::PermissionMode;
    use serde_json::json;
    use tempfile::TempDir;
    use tokio::sync::RwLock;

    #[test]
    fn durable_steering_promotes_once_after_the_completed_assistant_message() -> anyhow::Result<()>
    {
        let temp = TempDir::new()?;
        let db_path = temp.path().join("krusty.db");
        let manager = SessionManager::new(Database::new(&db_path)?);
        let session_id = manager.create_session("steering", Some("test-model"), None)?;
        let initial_user = ModelMessage {
            role: Role::User,
            content: vec![Content::Text {
                text: "start".into(),
            }],
        };
        let assistant = ModelMessage {
            role: Role::Assistant,
            content: vec![Content::Text {
                text: "first response".into(),
            }],
        };
        let steering_content = vec![Content::Text {
            text: "change direction".into(),
        }];
        let steering_json = serde_json::to_string(&steering_content)?;
        manager.save_message(
            &session_id,
            "user",
            &serde_json::to_string(&initial_user.content)?,
        )?;
        manager.queue_pending_steering(&session_id, "steer-1", &steering_json)?;
        manager.save_message(
            &session_id,
            "assistant",
            &serde_json::to_string(&assistant.content)?,
        )?;

        let (input_tx, input_rx) = mpsc::unbounded_channel();
        for _ in 0..2 {
            input_tx.send(LoopInput::Steer {
                pending_id: Some("steer-1".into()),
                content: steering_content.clone(),
            })?;
        }
        let mut inbox = LoopInputInbox::new(input_rx);
        inbox.collect_ready();
        let mut conversation = vec![initial_user, assistant];
        let mut ledger = ContextLedger::from_conversation(&conversation);

        let injected = inject_pending_steering(
            &mut inbox,
            &mut conversation,
            &mut ledger,
            &db_path,
            &session_id,
        );

        assert_eq!(
            injected.len(),
            1,
            "duplicate channel delivery must be idempotent"
        );
        assert!(no_tool_completion_should_continue(&injected));
        assert_eq!(conversation.len(), 3);
        let stored = manager.load_session_messages(&session_id)?;
        assert_eq!(
            stored
                .iter()
                .map(|(role, _)| role.as_str())
                .collect::<Vec<_>>(),
            vec!["user", "assistant", "user"],
            "promotion must place steering after the completed assistant response"
        );
        assert_eq!(
            stored
                .iter()
                .filter(|(_, content)| content.contains("change direction"))
                .count(),
            1,
            "steering must persist exactly once"
        );
        Ok(())
    }

    #[test]
    fn no_tool_completion_finishes_only_without_queued_steering() {
        assert!(!no_tool_completion_should_continue(&[]));
        assert!(no_tool_completion_should_continue(&[
            super::InjectedSteering {
                pending_id: Some("steer-1".into()),
                message: "keep going".into(),
            }
        ]));
    }

    #[test]
    fn autonomous_plan_is_detected_before_bundled_writes() {
        assert!(should_detect_plan_transition(
            WorkMode::Plan,
            PermissionMode::Autonomous,
            true,
        ));
        assert!(!should_detect_plan_transition(
            WorkMode::Plan,
            PermissionMode::Supervised,
            true,
        ));
        assert!(should_detect_plan_transition(
            WorkMode::Plan,
            PermissionMode::Supervised,
            false,
        ));
        assert!(!should_detect_plan_transition(
            WorkMode::Build,
            PermissionMode::Autonomous,
            false,
        ));
    }

    #[test]
    fn successful_mutation_requires_validation_and_successful_check_clears_it() {
        let mutation = AiToolCall {
            id: "edit-1".into(),
            name: "edit".into(),
            arguments: json!({"file_path": "src/lib.rs"}),
        };
        let validation = AiToolCall {
            id: "check-1".into(),
            name: "bash".into(),
            arguments: json!({"command": "cargo test -p krusty-core"}),
        };
        let success = |id: &str| Content::ToolResult {
            tool_use_id: id.into(),
            output: json!({"ok": true}),
            is_error: None,
        };
        let mut pending = false;

        assert!(update_validation_state(
            &mut pending,
            std::slice::from_ref(&mutation),
            &[success("edit-1")],
        ));
        assert!(pending);
        assert!(!update_validation_state(
            &mut pending,
            std::slice::from_ref(&validation),
            &[success("check-1")],
        ));
        assert!(!pending);
    }

    #[test]
    fn failed_validation_does_not_clear_pending_mutation() {
        let validation = AiToolCall {
            id: "check-1".into(),
            name: "bash".into(),
            arguments: json!({"command": "cargo check --workspace"}),
        };
        let mut pending = true;
        let failed = Content::ToolResult {
            tool_use_id: "check-1".into(),
            output: json!({"error": true}),
            is_error: Some(true),
        };

        assert!(!update_validation_state(
            &mut pending,
            &[validation],
            &[failed],
        ));
        assert!(pending);
    }

    #[test]
    fn successful_validation_removes_only_transient_validation_reminders() {
        let mut conversation = vec![
            ModelMessage {
                role: Role::System,
                content: vec![Content::Text {
                    text: "durable instruction".into(),
                }],
            },
            ModelMessage {
                role: Role::System,
                content: vec![Content::Text {
                    text: VALIDATION_REMINDER.into(),
                }],
            },
        ];

        remove_validation_reminders(&mut conversation);

        assert_eq!(conversation.len(), 1);
        assert!(matches!(
            &conversation[0].content[0],
            Content::Text { text } if text == "durable instruction"
        ));
    }

    #[test]
    fn chatgpt_codex_runtime_uses_conservative_context_window() {
        assert_eq!(effective_context_window_for_runtime(true, 400_000), 256_000);
        assert_eq!(effective_context_window_for_runtime(true, 128_000), 128_000);
        assert_eq!(
            effective_context_window_for_runtime(false, 400_000),
            400_000
        );
    }

    #[test]
    fn empty_stream_idle_retries_once_before_recovery() {
        assert!(should_retry_empty_stream_interruption(
            Some(&LoopStopReason::StreamIdleTimeout),
            None,
            false,
            false
        ));
        assert!(!should_retry_empty_stream_interruption(
            Some(&LoopStopReason::StreamIdleTimeout),
            None,
            true,
            false
        ));
        assert!(!should_retry_empty_stream_interruption(
            Some(&LoopStopReason::StreamIdleTimeout),
            None,
            false,
            true
        ));
        assert!(should_retry_empty_stream_interruption(
            Some(&LoopStopReason::ProviderError),
            Some("API error: 429 Too Many Requests - capacity"),
            false,
            false
        ));
        assert!(should_retry_empty_stream_interruption(
            Some(&LoopStopReason::ProviderError),
            Some("AI stream ended without a finish signal"),
            false,
            false
        ));
        assert!(!should_retry_empty_stream_interruption(
            Some(&LoopStopReason::ProviderError),
            Some("API error: 402 Payment Required - limit reached"),
            false,
            false
        ));
        assert!(!should_retry_empty_stream_interruption(
            Some(&LoopStopReason::ProviderError),
            Some("API error: 503 Service Unavailable"),
            true,
            false
        ));
    }

    #[test]
    fn semantic_empty_completion_retries_once_then_fails_visibly() {
        assert_eq!(
            empty_completion_action("", &[], false, false),
            EmptyCompletionAction::Retry
        );
        assert_eq!(
            empty_completion_action("  \n", &[], true, false),
            EmptyCompletionAction::Fail
        );
        assert_eq!(
            empty_completion_action("", &[], false, true),
            EmptyCompletionAction::Fail
        );
    }

    #[test]
    fn semantic_completion_with_visible_text_or_tool_call_needs_no_recovery() {
        let tool_call = AiToolCall {
            id: "call-1".into(),
            name: "bash".into(),
            arguments: json!({"command": "true"}),
        };

        assert_eq!(
            empty_completion_action("done", &[], false, false),
            EmptyCompletionAction::None
        );
        assert_eq!(
            empty_completion_action("", &[tool_call], false, false),
            EmptyCompletionAction::None
        );
    }

    #[test]
    fn stream_idle_timeout_is_terminal_but_not_active_processing() {
        assert_eq!(
            terminal_agent_state_after_interruption(&LoopStopReason::StreamIdleTimeout),
            "idle"
        );
        assert_eq!(
            terminal_agent_state_after_interruption(&LoopStopReason::ProviderError),
            "error"
        );
    }

    #[test]
    fn project_settings_cannot_elevate_supervised_permission_mode() {
        let settings = ProjectSettings {
            permission_mode: Some("autonomous".to_string()),
            ..Default::default()
        };

        let resolved = resolve_project_permission_mode(PermissionMode::Supervised, &settings);

        assert_eq!(resolved, PermissionMode::Supervised);
    }

    #[test]
    fn project_settings_can_restrict_autonomous_permission_mode() {
        let settings = ProjectSettings {
            permission_mode: Some("supervised".to_string()),
            ..Default::default()
        };

        let resolved = resolve_project_permission_mode(PermissionMode::Autonomous, &settings);

        assert_eq!(resolved, PermissionMode::Supervised);
    }

    #[test]
    fn project_settings_preserve_requested_autonomous_permission_mode() {
        let settings = ProjectSettings {
            permission_mode: Some("autonomous".to_string()),
            ..Default::default()
        };

        let resolved = resolve_project_permission_mode(PermissionMode::Autonomous, &settings);

        assert_eq!(resolved, PermissionMode::Autonomous);
    }

    #[test]
    fn finalize_explore_only_turn_returns_summary_for_successful_explore() -> anyhow::Result<()> {
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
            .ok_or_else(|| anyhow::anyhow!("explore should finalize"))?;

        assert!(summary.contains("Architecture review completed across 2 targets."));
        assert!(summary.contains("agent: Owns orchestration."));
        assert!(!summary.contains("Evidence examined: 15 tracked paths/files."));
        Ok(())
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
    fn inject_runtime_context_applies_mako_session_identity() -> anyhow::Result<()> {
        let temp = TempDir::new()?;
        let repo = temp.path();
        fs::create_dir_all(repo.join(".git"))?;
        fs::write(repo.join("AGENTS.md"), "repo instructions")?;
        fs::write(repo.join("MAKO.md"), "Always Swimming.")?;

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
        Ok(())
    }
}
