use std::convert::Infallible;

use axum::{
    extract::State,
    response::sse::{Event, Sse},
    Json,
};
use futures::stream::Stream;
use serde_json::json;

use krusty_core::agent::plan_handler::parse_plan_confirm_choice;
use krusty_core::agent::LoopInput;
use krusty_core::ai::types::{Content, ModelMessage, Role};
use krusty_core::plan::PlanManager;
use krusty_core::storage::{Database, PendingInteractionSnapshot, SessionType, WorkMode};
use krusty_core::tools::registry::PermissionMode;
use krusty_core::SessionManager;

use super::super::session_access::ensure_owned_session;
use super::session::{setup_chat_session, RequestedModel};
use super::stream::start_orchestrator_sse;
use super::tools::apply_thinking_config;
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::types::{ThinkingLevel, ToolApprovalRequest, ToolResultRequest};
use crate::AppState;

pub(super) async fn tool_result(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<ToolResultRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let mut ctx = setup_chat_session(
        &state,
        user.as_ref(),
        &req.session_id,
        RequestedModel::Unspecified,
        ThinkingLevel::Off,
        req.fast_mode,
        false,
        false,
    )
    .await?;

    // Plan confirmation is an internal orchestrator event, not a real tool call.
    // Don't add a ToolResult — instead add a user message to resume the conversation.
    if req.tool_call_id.starts_with("plan-confirm-") {
        let choice = parse_plan_confirm_choice(&req.result);
        let work_mode = if choice.as_deref() == Some("execute") {
            ctx.session_manager
                .update_session_work_mode(&req.session_id, WorkMode::Build)?;
            let user_content = vec![Content::Text {
                text:
                    "The plan has been approved. Begin executing the plan, starting with Task 1.1."
                        .to_string(),
            }];
            let user_json = serde_json::to_string(&user_content)?;
            ctx.conversation.push(ModelMessage {
                role: Role::User,
                content: user_content,
            });
            ctx.session_manager
                .save_message(&req.session_id, "user", &user_json)?;
            WorkMode::Build
        } else {
            if let Ok(plan_manager) = PlanManager::new((*state.db_path).clone()) {
                let _ = plan_manager.abandon_plan(&req.session_id);
            }
            let user_content = vec![Content::Text {
                text: "The plan has been abandoned. What would you like to do instead?".to_string(),
            }];
            let user_json = serde_json::to_string(&user_content)?;
            ctx.conversation.push(ModelMessage {
                role: Role::User,
                content: user_content,
            });
            ctx.session_manager
                .save_message(&req.session_id, "user", &user_json)?;
            ctx.work_mode
        };
        let permission_mode = continuation_permission_mode(&ctx);
        return start_orchestrator_sse(&state, ctx, work_mode, permission_mode, false).await;
    }

    let has_thinking = ctx.conversation.iter().any(|msg| {
        msg.content
            .iter()
            .any(|content| matches!(content, Content::Thinking { .. }))
    });
    if has_thinking {
        apply_thinking_config(&ctx.ai_client, ThinkingLevel::High, &mut ctx.options);
    }

    let merged = if let Some(last_msg) = ctx.conversation.last_mut() {
        if last_msg.role == Role::User
            && last_msg.content.iter().any(|content| {
                matches!(content, Content::ToolResult { tool_use_id, .. } if tool_use_id == req.tool_call_id.as_str())
            })
        {
            if let Some(output) = last_msg.content.iter_mut().find_map(|content| match content {
                Content::ToolResult {
                    tool_use_id, output, ..
                } if tool_use_id == req.tool_call_id.as_str() => Some(output),
                _ => None,
            }) {
                *output = serde_json::Value::String(req.result.clone());
            }
            let updated_json = serde_json::to_string(&last_msg.content)?;
            ctx.session_manager
                .update_last_message(&req.session_id, "user", &updated_json)?;
            true
        } else {
            false
        }
    } else {
        false
    };

    if !merged {
        let tool_result_content = vec![Content::ToolResult {
            tool_use_id: req.tool_call_id.clone(),
            output: serde_json::Value::String(req.result.clone()),
            is_error: None,
        }];
        let tool_result_json = serde_json::to_string(&tool_result_content)?;
        ctx.conversation.push(ModelMessage {
            role: Role::User,
            content: tool_result_content,
        });
        ctx.session_manager
            .save_message(&req.session_id, "user", &tool_result_json)?;
    }

    let work_mode = ctx.work_mode;
    let permission_mode = continuation_permission_mode(&ctx);
    start_orchestrator_sse(&state, ctx, work_mode, permission_mode, false).await
}

fn continuation_permission_mode(ctx: &super::session::ChatSessionContext) -> PermissionMode {
    if ctx.session_type == SessionType::Mako {
        return PermissionMode::Autonomous;
    }

    ctx.session_manager
        .load_recovery_state(&ctx.session_id)
        .ok()
        .flatten()
        .and_then(|state| state.permission_mode)
        .unwrap_or(PermissionMode::Supervised)
}

pub(super) async fn tool_approval(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<ToolApprovalRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    submit_tool_approval(&state, user.as_ref(), req).await
}

pub(crate) async fn submit_tool_approval(
    state: &AppState,
    user: Option<&CurrentUser>,
    req: ToolApprovalRequest,
) -> Result<Json<serde_json::Value>, AppError> {
    let session_manager = SessionManager::new(Database::new(&state.db_path)?);
    ensure_owned_session(&session_manager, &req.session_id, user)?;

    let sender = {
        let inputs = state.session_inputs.read().await;
        inputs.get(&req.session_id).cloned()
    };
    let Some(sender) = sender else {
        if recovery_has_pending_tool_approval(&session_manager, &req.session_id, &req.tool_call_id)?
        {
            return Err(AppError::Conflict(format!(
                "Session {} has recoverable pending tool approval {}, but the approval channel unavailable after reload or restart. Reload /sessions/{}/state for pending_interactions and retry once the session is active.",
                req.session_id, req.tool_call_id, req.session_id
            )));
        }
        return Err(AppError::NotFound("No active session".into()));
    };
    sender
        .send(LoopInput::ToolApproval {
            tool_call_id: req.tool_call_id,
            approved: req.approved,
        })
        .map_err(|_| {
            AppError::Conflict(format!(
                "Session {} is no longer accepting tool approvals",
                req.session_id
            ))
        })?;
    Ok(Json(json!({"status": "ok"})))
}

fn recovery_has_pending_tool_approval(
    session_manager: &SessionManager,
    session_id: &str,
    tool_call_id: &str,
) -> Result<bool, AppError> {
    let Some(recovery) = session_manager.load_recovery_state(session_id)? else {
        return Ok(false);
    };

    Ok(recovery.pending_interactions.iter().any(|pending| {
        matches!(
            pending,
            PendingInteractionSnapshot::ToolApproval { tool_call }
                if tool_call.id == tool_call_id
        )
    }))
}
