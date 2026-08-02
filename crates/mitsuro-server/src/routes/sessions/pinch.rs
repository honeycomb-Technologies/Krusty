use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};

use mitsuro_core::agent::{
    effective_context_window_for_runtime, estimate_rendered_request_tokens, inject_context,
    run_compaction_pipeline, CompactionManager, CompactionRequest, CompactionTrigger,
};
use mitsuro_core::ai::client::CallOptions;
use mitsuro_core::storage::WorkspaceMode;
use std::path::Path as StdPath;

use super::{load_owned_session, open_session_manager, request_workspace_scope};
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::types::{PinchRequest, PinchResponse};
use crate::utils::messages::parse_stored_model_messages;
use crate::utils::workspace::{resolve_optional_workspace_path, resolve_session_working_dir};
use crate::AppState;

/// Compact a session in place (manual pinch).
pub(super) async fn pinch_session(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<PinchRequest>,
) -> Result<Json<PinchResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    let source_session = load_owned_session(&session_manager, &id, user.as_ref())?;
    if source_session.session_type == mitsuro_core::storage::SessionType::Hive {
        return Err(AppError::Conflict(
            "Hive compaction is coordinated by its background service; /sessions/:id/pinch is unavailable for Hive sessions".into(),
        ));
    }
    let _session_guard = state
        .try_lock_session(&id)
        .await
        .ok_or_else(|| AppError::Conflict(format!("Session {id} is busy")))?;
    let workspace_scope = request_workspace_scope(&state, user.as_ref());

    let raw_messages = session_manager.load_session_messages(&id)?;
    let messages = parse_stored_model_messages(&id, raw_messages, "pinch context");

    if messages.is_empty() {
        return Err(AppError::BadRequest(
            "Cannot compact session with no messages".to_string(),
        ));
    }

    let working_dir = resolve_session_working_dir(
        source_session.working_dir.as_deref(),
        &workspace_scope.base_dir,
        &workspace_scope.allowed_root,
    )?;
    let legacy_relative_workspace =
        has_relative_workspace_path(source_session.working_dir.as_deref())
            || has_relative_workspace_path(source_session.project_dir.as_deref());
    let resolved_working_dir = working_dir.to_string_lossy().into_owned();
    let resolved_project_dir =
        if legacy_relative_workspace && source_session.workspace_mode != WorkspaceMode::Neutral {
            Some(
                resolve_optional_workspace_path(
                    source_session.project_dir.as_deref(),
                    &workspace_scope.base_dir,
                    &workspace_scope.allowed_root,
                )?
                .unwrap_or_else(|| resolved_working_dir.clone()),
            )
        } else {
            None
        };

    let summary_client = if let Some(key) = source_session.model_key.as_ref() {
        state
            .resolve_ai_client_for_key_for_user(key, source_session.user_id.as_deref())
            .await
    } else {
        state
            .resolve_ai_client_for_user(
                source_session.model.as_deref(),
                source_session.user_id.as_deref(),
            )
            .await
    };
    let summary_model = summary_client
        .as_ref()
        .map(|client| client.resolved_model().wire_model_id.as_str())
        .or(source_session.model.as_deref());

    let (compaction_manager, request_budget) = if let Some(client) = summary_client.as_ref() {
        let model = client.resolved_model().wire_model_id.as_str();
        let effective_window = effective_context_window_for_runtime(
            client.config().uses_chatgpt_codex_format(),
            client.resolved_model().capabilities.context_window,
        );
        let manager = CompactionManager::for_model(
            client.provider_id(),
            client.config().api_format,
            model,
            effective_window,
        );
        let session_type = match source_session.session_type {
            mitsuro_core::storage::SessionType::Chat => "chat",
            mitsuro_core::storage::SessionType::Code => "code",
            mitsuro_core::storage::SessionType::Hive => "hive",
        };
        let context_project_dir = (source_session.workspace_mode != WorkspaceMode::Neutral)
            .then_some(working_dir.as_path());
        let with_context = inject_context(
            &messages,
            &state.db_path,
            &id,
            &working_dir,
            context_project_dir,
            source_session.work_mode,
            state.skills_manager.as_ref(),
            Some(model),
            Some(session_type),
            None,
            source_session.user_id.as_deref(),
        );
        let tools = state.tool_registry.get_ai_tools_all().await;
        let options = CallOptions {
            tools: (!tools.is_empty()).then_some(tools),
            enable_caching: true,
            session_id: Some(id.clone()),
            codex_parallel_tool_calls: true,
            ..Default::default()
        };
        let rendered = estimate_rendered_request_tokens(client, &with_context, &options);
        (
            manager,
            Some(rendered.compaction_budget(rendered.total_tokens)),
        )
    } else {
        (
            CompactionManager::for_model(
                mitsuro_core::ai::providers::ProviderId::MiniMax,
                mitsuro_core::ai::models::ApiFormat::Anthropic,
                mitsuro_core::constants::ai::DEFAULT_MODEL,
                mitsuro_core::constants::ai::CONTEXT_WINDOW_TOKENS,
            ),
            None,
        )
    };

    let compaction_result = run_compaction_pipeline(CompactionRequest {
        db_path: &state.db_path,
        session_id: &id,
        conversation: &messages,
        working_dir: &working_dir,
        ai_client: summary_client.as_ref().map(|client| client.as_ref()),
        model: summary_model,
        trigger: CompactionTrigger::Manual {
            preservation_hints: req.preservation_hints,
            direction: req.direction,
        },
        compaction_manager,
        request_budget,
        last_usage_prompt_tokens: None,
        messages_after_usage: 0,
        summary_override: None,
        project_dir: resolved_project_dir.as_deref(),
        user_id: source_session.user_id.as_deref(),
    })
    .await
    .map_err(|error| {
        let message = format!("Compaction failed: {error}");
        if message.contains("stale") || message.contains("changed while compaction") {
            AppError::Conflict(message)
        } else {
            AppError::Internal(message)
        }
    })?;

    if legacy_relative_workspace {
        let (working_dir, project_dir) = match source_session.workspace_mode {
            WorkspaceMode::Neutral => (None, None),
            WorkspaceMode::Selected | WorkspaceMode::Created => (
                Some(resolved_working_dir.as_str()),
                resolved_project_dir.as_deref(),
            ),
        };
        session_manager.update_session_workspace_contract(
            &id,
            working_dir,
            project_dir,
            source_session.workspace_mode,
        )?;
    }

    let session = session_manager
        .get_session(&id)?
        .ok_or_else(|| AppError::Internal("Failed to fetch compacted session".to_string()))?;

    Ok(Json(PinchResponse {
        session: crate::types::SessionResponse::from_session(
            session,
            crate::legacy_identity::SessionWireFormat::from_headers(&headers),
        ),
        summary: compaction_result.summary.work_summary,
        key_decisions: compaction_result.summary.key_decisions,
        pending_tasks: compaction_result.summary.pending_tasks,
        estimated_tokens_before: Some(compaction_result.estimated_tokens_before),
        estimated_tokens_after: Some(compaction_result.estimated_tokens_after),
        replaced_messages: Some(compaction_result.replaced_messages),
        checkpoint_id: Some(compaction_result.checkpoint_id),
        compaction_count: Some(compaction_result.compaction_count),
    }))
}

fn has_relative_workspace_path(path: Option<&str>) -> bool {
    path.map(str::trim)
        .filter(|path| !path.is_empty())
        .map(|path| !StdPath::new(path).is_absolute())
        .unwrap_or(false)
}
