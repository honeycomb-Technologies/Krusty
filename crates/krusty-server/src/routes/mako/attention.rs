use std::collections::HashMap;

use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use krusty_core::agent::DelegatedRunStage;
use krusty_core::storage::{
    Database, DelegatedRunStore, MakoAttentionItemState, MakoAttentionStateStore,
    MakoRuntimeStateStatus, MessageStore, RuntimeTraceEvent, RuntimeTraceStore, SessionType,
    StoredMessageRecord,
};

use super::super::session_access::{current_user_id, load_owned_session_of_type};
use super::current::{
    build_mako_current_response, parse_timestamp, MakoCurrentResponse, MakoCurrentRunSummary,
};
use super::{open_session_manager, OkResponse};
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::AppState;

#[derive(Debug, Deserialize)]
pub(super) struct AttentionQuery {
    pub(super) thread_session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AttentionReadRequest {
    pub(super) read: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct AttentionClearRequest {
    pub(super) cleared: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct MakoAttentionResponse {
    pub(super) items: Vec<MakoAttentionItemSummary>,
    pub(super) unread_count: usize,
    pub(super) badge_count: usize,
}

#[derive(Debug, Serialize, Clone)]
pub(super) struct MakoAttentionItemSummary {
    pub(super) id: String,
    pub(super) kind: String,
    pub(super) section: String,
    pub(super) title: String,
    pub(super) summary: String,
    pub(super) detail: String,
    pub(super) created_at: String,
    pub(super) read: bool,
    pub(super) cleared: bool,
    pub(super) active: bool,
    pub(super) session_id: Option<String>,
    pub(super) run_id: Option<String>,
    pub(super) project_dir: Option<String>,
    pub(super) target_branch: Option<String>,
    pub(super) tool_call_id: Option<String>,
    pub(super) thread_session_id: Option<String>,
    pub(super) thread_message_id: Option<String>,
}

pub(super) async fn attention(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Query(query): Query<AttentionQuery>,
) -> Result<Json<MakoAttentionResponse>, AppError> {
    let thread_context =
        load_attention_thread_context(&state, user.as_ref(), query.thread_session_id.as_deref())?;
    let current = build_mako_current_response(&state, user.as_ref()).await?;
    let db = Database::new(&state.db_path)?;
    let store = MakoAttentionStateStore::new(&db);
    let state_by_id = store.list_for_user(current_user_id(user.as_ref()))?;
    let trace_db = Database::new(&state.db_path)?;
    let trace_store = RuntimeTraceStore::new(&trace_db);
    let delegated_store = DelegatedRunStore::new(Database::new(&state.db_path)?);
    let items = build_attention_items(
        &current,
        &state_by_id,
        thread_context.as_ref(),
        &trace_store,
        &delegated_store,
    )?;
    let unread_count = items.iter().filter(|item| !item.read).count();
    let badge_count = items
        .iter()
        .filter(|item| {
            !item.read
                && item.active
                && matches!(
                    item.kind.as_str(),
                    "approval_required" | "input_required" | "run_failed" | "run_stalled"
                )
        })
        .count();

    Ok(Json(MakoAttentionResponse {
        items,
        unread_count,
        badge_count,
    }))
}

pub(super) async fn set_attention_read(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    Json(request): Json<AttentionReadRequest>,
) -> Result<Json<OkResponse>, AppError> {
    let db = Database::new(&state.db_path)?;
    let store = MakoAttentionStateStore::new(&db);
    store.set_read(current_user_id(user.as_ref()), &id, request.read)?;
    Ok(Json(OkResponse { ok: true }))
}

pub(super) async fn set_attention_cleared(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    Json(request): Json<AttentionClearRequest>,
) -> Result<Json<OkResponse>, AppError> {
    let db = Database::new(&state.db_path)?;
    let store = MakoAttentionStateStore::new(&db);
    store.set_cleared(current_user_id(user.as_ref()), &id, request.cleared)?;
    Ok(Json(OkResponse { ok: true }))
}

#[derive(Debug)]
struct AttentionThreadContext {
    session_id: String,
    messages: Vec<AttentionThreadMessage>,
}

#[derive(Debug)]
struct AttentionThreadMessage {
    created_at: chrono::DateTime<chrono::Utc>,
    role: String,
    message_id: String,
}

fn load_attention_thread_context(
    state: &AppState,
    user: Option<&CurrentUser>,
    thread_session_id: Option<&str>,
) -> Result<Option<AttentionThreadContext>, AppError> {
    let Some(thread_session_id) = thread_session_id else {
        return Ok(None);
    };

    let session_manager = open_session_manager(state)?;
    let session = load_owned_session_of_type(
        &session_manager,
        thread_session_id,
        SessionType::Mako,
        "Mako",
        user,
    )?;
    let db = Database::new(&state.db_path)?;
    let records = MessageStore::new(&db).load_session_message_records(thread_session_id)?;

    Ok(Some(AttentionThreadContext {
        session_id: session.id,
        messages: build_attention_thread_messages(&records),
    }))
}

fn build_attention_thread_messages(records: &[StoredMessageRecord]) -> Vec<AttentionThreadMessage> {
    let mut messages = Vec::new();

    for record in records {
        let Some(parsed) = parse_attention_thread_message(record) else {
            continue;
        };

        let index = messages.len();
        let role = parsed.role;
        messages.push(AttentionThreadMessage {
            created_at: parsed.created_at,
            role: role.clone(),
            message_id: build_attention_thread_message_id(
                index,
                role.as_str(),
                parsed.content_len,
                parsed.thinking_len,
                parsed.first_tool_id.as_deref(),
            ),
        });
    }

    messages
}

#[derive(Debug)]
struct ParsedAttentionThreadMessage {
    role: String,
    created_at: chrono::DateTime<chrono::Utc>,
    content_len: usize,
    thinking_len: usize,
    first_tool_id: Option<String>,
}

fn parse_attention_thread_message(
    record: &StoredMessageRecord,
) -> Option<ParsedAttentionThreadMessage> {
    let created_at = parse_timestamp(record.created_at.as_str())?;
    let value: Value = serde_json::from_str(record.content_json.as_str()).ok()?;
    let content_array = value.as_array()?;

    let mut content = String::new();
    let mut thinking = String::new();
    let mut first_tool_id: Option<String> = None;

    for block in content_array {
        let Some(object) = block.as_object() else {
            continue;
        };
        let block_type = object.get("type").and_then(Value::as_str);

        if block_type == Some("text") || (block_type.is_none() && object.contains_key("text")) {
            if let Some(text) = object.get("text").and_then(Value::as_str) {
                if !text.is_empty() {
                    if !content.is_empty() {
                        content.push('\n');
                    }
                    content.push_str(text);
                }
            }
            continue;
        }

        if block_type == Some("thinking") || object.contains_key("thinking") {
            if let Some(text) = object.get("thinking").and_then(Value::as_str) {
                if !text.is_empty() {
                    if !thinking.is_empty() {
                        thinking.push_str("\n\n");
                    }
                    thinking.push_str(text);
                }
            }
            continue;
        }

        let has_tool_fields = object.contains_key("id")
            && object.contains_key("name")
            && object.contains_key("input");
        if (block_type == Some("tool_use") || has_tool_fields) && first_tool_id.is_none() {
            first_tool_id = object
                .get("id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
        }
    }

    if content.trim().is_empty() && thinking.trim().is_empty() && first_tool_id.is_none() {
        return None;
    }

    Some(ParsedAttentionThreadMessage {
        role: record.role.clone(),
        created_at,
        content_len: content.encode_utf16().count(),
        thinking_len: thinking.encode_utf16().count(),
        first_tool_id,
    })
}

fn build_attention_thread_message_id(
    index: usize,
    role: &str,
    content_len: usize,
    thinking_len: usize,
    first_tool_id: Option<&str>,
) -> String {
    if let Some(first_tool_id) = first_tool_id {
        return format!("stored-{index}-{role}-{first_tool_id}");
    }

    format!("stored-{index}-{role}-{content_len}-{thinking_len}")
}

fn build_attention_items(
    current: &MakoCurrentResponse,
    state_by_id: &HashMap<String, MakoAttentionItemState>,
    thread_context: Option<&AttentionThreadContext>,
    trace_store: &RuntimeTraceStore<'_>,
    delegated_store: &DelegatedRunStore,
) -> Result<Vec<MakoAttentionItemSummary>, AppError> {
    let mut items = Vec::new();

    for approval in &current.approvals {
        let item = apply_attention_state(
            MakoAttentionItemSummary {
                id: format!("approval:{}:{}", approval.session_id, approval.tool_call_id),
                kind: "approval_required".to_string(),
                section: "needs_action".to_string(),
                title: "Approval request".to_string(),
                summary: format!(
                    "{} in {}",
                    approval.tool_name,
                    project_label(approval.project_dir.as_deref())
                ),
                detail: format!(
                    "Mako is waiting for approval to continue {}.",
                    approval.session_title
                ),
                created_at: approval.requested_at.clone(),
                read: false,
                cleared: false,
                active: true,
                session_id: Some(approval.session_id.clone()),
                run_id: Some(approval.session_id.clone()),
                project_dir: approval.project_dir.clone(),
                target_branch: approval.target_branch.clone(),
                tool_call_id: Some(approval.tool_call_id.clone()),
                thread_session_id: thread_context.map(|context| context.session_id.clone()),
                thread_message_id: thread_context.and_then(|context| {
                    find_thread_message_id(context, approval.requested_at.as_str())
                }),
            },
            state_by_id.get(&format!(
                "approval:{}:{}",
                approval.session_id, approval.tool_call_id
            )),
        );
        items.push(item);
    }

    for run in &current.runs {
        for item in build_run_attention_items(run, thread_context, trace_store, delegated_store)? {
            if let Some(item) = maybe_keep_attention_item(item, state_by_id) {
                items.push(item);
            }
        }
    }

    items.sort_by(|left, right| {
        attention_section_rank(left.section.as_str())
            .cmp(&attention_section_rank(right.section.as_str()))
            .then_with(|| left.read.cmp(&right.read))
            .then_with(|| right.created_at.cmp(&left.created_at))
    });
    Ok(items)
}

fn maybe_keep_attention_item(
    item: MakoAttentionItemSummary,
    state_by_id: &HashMap<String, MakoAttentionItemState>,
) -> Option<MakoAttentionItemSummary> {
    let item_id = item.id.clone();
    let applied = apply_attention_state(item, state_by_id.get(item_id.as_str()));
    if applied.cleared && applied.section == "updates" {
        return None;
    }
    Some(applied)
}

fn apply_attention_state(
    mut item: MakoAttentionItemSummary,
    state: Option<&MakoAttentionItemState>,
) -> MakoAttentionItemSummary {
    if let Some(state) = state {
        item.read = state.read;
        item.cleared = state.cleared;
    }

    if item.section == "needs_action" && item.active {
        item.cleared = false;
    }

    item
}

fn build_run_attention_items(
    run: &MakoCurrentRunSummary,
    thread_context: Option<&AttentionThreadContext>,
    trace_store: &RuntimeTraceStore<'_>,
    delegated_store: &DelegatedRunStore,
) -> Result<Vec<MakoAttentionItemSummary>, AppError> {
    let trace_events = trace_store.list_events(&run.session_id, Some(200))?;
    let delegated_runs = delegated_store.list_runs_for_session(&run.session_id, 25)?;
    let mut items = Vec::new();

    if let Some(item) = build_run_action_attention_item(run, thread_context) {
        items.push(item);
    }

    if let Some(item) = build_scheduled_started_attention_item(run, &trace_events, thread_context) {
        items.push(item);
    }

    if let Some(item) =
        build_delegated_completion_attention_item(run, &delegated_runs, thread_context)
    {
        items.push(item);
    }

    if let Some(item) = build_run_completion_attention_item(run, &trace_events, thread_context) {
        items.push(item);
    }

    Ok(items)
}

fn build_run_action_attention_item(
    run: &MakoCurrentRunSummary,
    thread_context: Option<&AttentionThreadContext>,
) -> Option<MakoAttentionItemSummary> {
    let run_id = run.session_id.clone();
    let project_dir = run.project_dir.clone();
    let target_branch = run.target_branch.clone();
    let thread_session_id = thread_context.map(|context| context.session_id.clone());
    let thread_message_id =
        thread_context.and_then(|context| find_thread_message_id(context, run.updated_at.as_str()));

    let diagnostic = run.diagnostic.as_ref()?;
    if !matches!(
        diagnostic.kind.as_str(),
        "awaiting_input"
            | "failed"
            | "stalled_stream"
            | "stale_active"
            | "stale_waiting"
            | "stale_queued"
    ) {
        return None;
    }

    let (kind, title) = match diagnostic.kind.as_str() {
        "awaiting_input" => ("input_required", "Reply needed"),
        "failed" => ("run_failed", "Run error"),
        _ => ("run_stalled", "Run needs attention"),
    };

    Some(MakoAttentionItemSummary {
        id: format!("run:{run_id}:{kind}"),
        kind: kind.to_string(),
        section: "needs_action".to_string(),
        title: title.to_string(),
        summary: run.title.clone(),
        detail: diagnostic.detail.clone(),
        created_at: run.updated_at.clone(),
        read: false,
        cleared: false,
        active: true,
        session_id: Some(run_id.clone()),
        run_id: Some(run_id),
        project_dir,
        target_branch,
        tool_call_id: None,
        thread_session_id,
        thread_message_id,
    })
}

fn build_scheduled_started_attention_item(
    run: &MakoCurrentRunSummary,
    trace_events: &[RuntimeTraceEvent],
    thread_context: Option<&AttentionThreadContext>,
) -> Option<MakoAttentionItemSummary> {
    if !run_has_scheduled_origin(run) || run_is_completed(run) || run_is_pending_schedule(run) {
        return None;
    }

    let source_run_id = current_or_latest_trace_run_id(run, trace_events)?;
    let created_at =
        run_started_at(trace_events, &source_run_id).unwrap_or_else(|| run.updated_at.clone());
    let project = project_label(run.project_dir.as_deref());

    Some(MakoAttentionItemSummary {
        id: format!("run:{}:scheduled_started:{}", run.session_id, source_run_id),
        kind: "scheduled_run_started".to_string(),
        section: "updates".to_string(),
        title: "Scheduled run started".to_string(),
        summary: format!("Started on schedule: {}", run.title),
        detail: format!("Mako started this scheduled run in {project}."),
        created_at: created_at.clone(),
        read: false,
        cleared: false,
        active: false,
        session_id: Some(run.session_id.clone()),
        run_id: Some(run.session_id.clone()),
        project_dir: run.project_dir.clone(),
        target_branch: run.target_branch.clone(),
        tool_call_id: None,
        thread_session_id: thread_context.map(|context| context.session_id.clone()),
        thread_message_id: thread_context
            .and_then(|context| find_thread_message_id(context, created_at.as_str())),
    })
}

fn build_delegated_completion_attention_item(
    run: &MakoCurrentRunSummary,
    delegated_runs: &[krusty_core::storage::DelegatedRunRecord],
    thread_context: Option<&AttentionThreadContext>,
) -> Option<MakoAttentionItemSummary> {
    let delegated = delegated_runs.iter().find(|record| {
        matches!(
            record.stage,
            DelegatedRunStage::Complete | DelegatedRunStage::Degraded
        ) && record.completed_at.is_some()
    })?;
    let created_at = delegated.completed_at?.to_rfc3339();
    let scope_label = delegated_scope_label(&delegated.target_scope);
    let role_label = delegated_role_label(delegated.role.clone());
    let status_suffix = if delegated.stage == DelegatedRunStage::Degraded {
        " with warnings"
    } else {
        ""
    };
    let detail = delegated_detail(delegated).unwrap_or_else(|| {
        if scope_label.is_empty() {
            format!(
                "{role_label} finished a delegated task{status_suffix} for {}.",
                run.title
            )
        } else {
            format!("{role_label} finished{status_suffix} for {scope_label}.")
        }
    });
    let summary = if scope_label.is_empty() {
        format!("{role_label} finished a delegated task")
    } else {
        format!("{role_label} finished: {scope_label}")
    };

    Some(MakoAttentionItemSummary {
        id: format!("delegated:{}:completed", delegated.delegated_run_id),
        kind: "delegated_task_completed".to_string(),
        section: "updates".to_string(),
        title: "Crew update".to_string(),
        summary,
        detail,
        created_at: created_at.clone(),
        read: false,
        cleared: false,
        active: false,
        session_id: Some(run.session_id.clone()),
        run_id: Some(run.session_id.clone()),
        project_dir: run.project_dir.clone(),
        target_branch: run.target_branch.clone(),
        tool_call_id: delegated.parent_tool_call_id.clone(),
        thread_session_id: thread_context.map(|context| context.session_id.clone()),
        thread_message_id: thread_context
            .and_then(|context| find_thread_message_id(context, created_at.as_str())),
    })
}

fn build_run_completion_attention_item(
    run: &MakoCurrentRunSummary,
    trace_events: &[RuntimeTraceEvent],
    thread_context: Option<&AttentionThreadContext>,
) -> Option<MakoAttentionItemSummary> {
    if is_failed_attention_run(run) || !run_is_completed(run) {
        return None;
    }

    let completed_run_id =
        latest_trace_run_id(trace_events).unwrap_or_else(|| run.session_id.clone());
    let created_at = run_finished_at(trace_events, completed_run_id.as_str())
        .unwrap_or_else(|| run.updated_at.clone());
    let scheduled = run_has_scheduled_origin(run);

    Some(MakoAttentionItemSummary {
        id: if scheduled {
            format!(
                "run:{}:scheduled_completed:{}",
                run.session_id, completed_run_id
            )
        } else {
            format!("run:{}:completed:{}", run.session_id, completed_run_id)
        },
        kind: if scheduled {
            "scheduled_run_completed".to_string()
        } else {
            "run_completed".to_string()
        },
        section: "updates".to_string(),
        title: if scheduled {
            "Scheduled run finished".to_string()
        } else {
            "Run completed".to_string()
        },
        summary: if scheduled {
            format!("Finished on schedule: {}", run.title)
        } else {
            run.title.clone()
        },
        detail: if scheduled {
            format!(
                "{} task{} completed for this scheduled run in {}.",
                run.completed_tasks,
                if run.completed_tasks == 1 { "" } else { "s" },
                project_label(run.project_dir.as_deref())
            )
        } else {
            format!(
                "{} task{} completed in {}.",
                run.completed_tasks,
                if run.completed_tasks == 1 { "" } else { "s" },
                project_label(run.project_dir.as_deref())
            )
        },
        created_at: created_at.clone(),
        read: false,
        cleared: false,
        active: false,
        session_id: Some(run.session_id.clone()),
        run_id: Some(run.session_id.clone()),
        project_dir: run.project_dir.clone(),
        target_branch: run.target_branch.clone(),
        tool_call_id: None,
        thread_session_id: thread_context.map(|context| context.session_id.clone()),
        thread_message_id: thread_context
            .and_then(|context| find_thread_message_id(context, created_at.as_str())),
    })
}

fn run_has_scheduled_origin(run: &MakoCurrentRunSummary) -> bool {
    matches!(
        run.runtime
            .as_ref()
            .and_then(|runtime| runtime.last_wake_reason.as_deref()),
        Some("scheduled_dispatch" | "manual_schedule")
    )
}

fn run_is_pending_schedule(run: &MakoCurrentRunSummary) -> bool {
    matches!(
        run.runtime.as_ref(),
        Some(runtime)
            if runtime.status == MakoRuntimeStateStatus::Sleeping
                && runtime.sleep_reason.as_deref() == Some("scheduled")
    )
}

fn current_or_latest_trace_run_id(
    run: &MakoCurrentRunSummary,
    trace_events: &[RuntimeTraceEvent],
) -> Option<String> {
    run.runtime
        .as_ref()
        .and_then(|runtime| runtime.current_run_id.clone())
        .or_else(|| latest_trace_run_id(trace_events))
        .or_else(|| Some(run.session_id.clone()))
}

fn latest_trace_run_id(trace_events: &[RuntimeTraceEvent]) -> Option<String> {
    trace_events.last().map(|event| event.run_id.clone())
}

fn run_started_at(trace_events: &[RuntimeTraceEvent], run_id: &str) -> Option<String> {
    trace_events
        .iter()
        .find(|event| event.run_id == run_id)
        .map(|event| event.created_at.clone())
}

fn run_finished_at(trace_events: &[RuntimeTraceEvent], run_id: &str) -> Option<String> {
    trace_events
        .iter()
        .rev()
        .find(|event| event.run_id == run_id && event.event_type == "finished")
        .or_else(|| {
            trace_events
                .iter()
                .rev()
                .find(|event| event.run_id == run_id)
        })
        .map(|event| event.created_at.clone())
}

fn delegated_role_label(role: krusty_core::storage::DelegatedRunRole) -> &'static str {
    match role {
        krusty_core::storage::DelegatedRunRole::Explore => "Explore",
        krusty_core::storage::DelegatedRunRole::Build => "Build",
        krusty_core::storage::DelegatedRunRole::Planner => "Plan",
        krusty_core::storage::DelegatedRunRole::Verifier => "Verify",
    }
}

fn delegated_scope_label(scopes: &[krusty_core::storage::DelegatedRunScope]) -> String {
    let Some(scope) = scopes.first() else {
        return String::new();
    };

    if !scope.label.trim().is_empty() {
        return scope.label.trim().to_string();
    }

    if !scope.path.trim().is_empty() {
        return scope.path.trim().to_string();
    }

    scope.kind.trim().to_string()
}

fn delegated_detail(record: &krusty_core::storage::DelegatedRunRecord) -> Option<String> {
    if let Some(review) = record
        .human_review
        .as_deref()
        .map(str::trim)
        .filter(|review| !review.is_empty())
    {
        return Some(review.to_string());
    }

    record
        .snapshot
        .as_ref()
        .and_then(|snapshot| {
            snapshot
                .agents
                .iter()
                .find_map(|agent| agent.completion_summary.as_deref())
        })
        .map(str::trim)
        .filter(|summary| !summary.is_empty())
        .map(ToString::to_string)
}

fn project_label(path: Option<&str>) -> String {
    let Some(path) = path else {
        return "No project selected".to_string();
    };

    let parts = path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() <= 2 {
        return path.to_string();
    }
    parts[parts.len().saturating_sub(2)..].join("/")
}

fn is_failed_attention_run(run: &MakoCurrentRunSummary) -> bool {
    run.runtime
        .as_ref()
        .map(|runtime| runtime.status == MakoRuntimeStateStatus::Error)
        .unwrap_or(false)
        || run.agent_state == "error"
}

fn run_is_completed(run: &MakoCurrentRunSummary) -> bool {
    if is_failed_attention_run(run) {
        return false;
    }

    let has_open_tasks = run.pending_tasks + run.in_progress_tasks + run.blocked_tasks > 0;
    !has_open_tasks
        && !matches!(
            run.runtime.as_ref().map(|runtime| runtime.status),
            Some(MakoRuntimeStateStatus::Running)
                | Some(MakoRuntimeStateStatus::Sleeping)
                | Some(MakoRuntimeStateStatus::Paused)
                | Some(MakoRuntimeStateStatus::AwaitingInput)
        )
}

fn attention_section_rank(section: &str) -> usize {
    match section {
        "needs_action" => 0,
        _ => 1,
    }
}

fn find_thread_message_id(context: &AttentionThreadContext, created_at: &str) -> Option<String> {
    let target = parse_timestamp(created_at)?;
    let mut fallback_any: Option<&AttentionThreadMessage> = None;
    let mut fallback_assistant: Option<&AttentionThreadMessage> = None;

    for message in context.messages.iter().rev() {
        if message.created_at <= target {
            fallback_any = Some(message);
            if message.role == "assistant" {
                fallback_assistant = Some(message);
                break;
            }
        }
    }

    fallback_assistant
        .or(fallback_any)
        .or_else(|| {
            context
                .messages
                .iter()
                .rev()
                .find(|message| message.role == "assistant")
        })
        .or_else(|| context.messages.last())
        .map(|message| message.message_id.clone())
}
