//! Hive Group rooms: CRUD, membership, room messages, turn state, and a
//! room event stream.
//!
//! Ownership is exact-owner everywhere (NULL owner = the local profile,
//! never a wildcard). CRUD and reads go direct to storage like Workers;
//! message sends and stop are run-triggering mutations and fail closed onto
//! the daemon control plane. Delete archives — a group's timeline and its
//! Workers are never destroyed by leaving a room.

use std::convert::Infallible;
use std::time::Duration;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::sse::{Event, KeepAlive, Sse},
    Json,
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use mitsuro_core::storage::{
    Database, HiveGroup, HiveGroupExecutionMode, HiveGroupMessage, HiveGroupStatus, HiveGroupStore,
    HiveGroupTurn, HiveGroupUpdate, HiveWorker, HiveWorkerStatus, HiveWorkerStore, NewHiveGroup,
    SessionType, WorkspaceMode, MAX_HIVE_GROUP_MESSAGE_BYTES,
};

use super::super::session_access::current_user_id;
use super::{idempotency_key_from_headers, open_session_manager, OkResponse};
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::utils::text::trimmed_nonempty;
use crate::AppState;

const DEFAULT_MESSAGE_PAGE: usize = 100;
const MAX_MESSAGE_PAGE: usize = 500;
/// DB-tail cadence for the room event stream. Each observer tails its own
/// cursor, so a slow client only delays itself and can never stall or lag the
/// scheduler loop; the cursor makes loss impossible, which is why no Lagged
/// signal is needed here.
const GROUP_EVENT_POLL: Duration = Duration::from_millis(750);
const GROUP_EVENT_STREAM_BUFFER: usize = 256;

#[derive(Debug, Serialize)]
pub(super) struct HiveGroupMemberSummary {
    pub(super) worker_id: String,
    pub(super) slug: String,
    pub(super) display_name: String,
    pub(super) avatar_color: Option<String>,
    pub(super) model: Option<String>,
    pub(super) provider: Option<String>,
    pub(super) status: String,
}

#[derive(Debug, Serialize)]
pub(super) struct HiveGroupSummary {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) execution_mode: String,
    pub(super) max_rounds: u32,
    pub(super) max_member_messages_per_turn: u32,
    pub(super) parallelism: u32,
    pub(super) context_window_messages: u32,
    pub(super) status: String,
    pub(super) default_assignee_worker_id: Option<String>,
    pub(super) members: Vec<HiveGroupMemberSummary>,
    pub(super) active_turn_id: Option<String>,
    pub(super) latest_seq: i64,
    pub(super) created_at: String,
    pub(super) updated_at: String,
}

#[derive(Debug, Serialize)]
pub(super) struct HiveGroupsResponse {
    pub(super) groups: Vec<HiveGroupSummary>,
}

#[derive(Debug, Serialize)]
pub(super) struct HiveGroupTurnView {
    pub(super) id: String,
    pub(super) group_id: String,
    pub(super) trigger_message_id: String,
    pub(super) execution_mode: String,
    pub(super) status: String,
    pub(super) speaker_plan: Vec<String>,
    pub(super) next_speaker_index: u32,
    pub(super) member_outcomes: Option<serde_json::Value>,
    pub(super) started_at: String,
    pub(super) finished_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct HiveGroupDetailResponse {
    #[serde(flatten)]
    pub(super) group: HiveGroupSummary,
    pub(super) active_turn: Option<HiveGroupTurnView>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateGroupRequest {
    pub(super) title: String,
    pub(super) execution_mode: Option<HiveGroupExecutionMode>,
    pub(super) max_rounds: Option<u32>,
    pub(super) max_member_messages_per_turn: Option<u32>,
    pub(super) parallelism: Option<u32>,
    pub(super) context_window_messages: Option<u32>,
    pub(super) default_assignee_worker_id: Option<String>,
    pub(super) member_worker_ids: Vec<String>,
}

/// Partial update: absent fields keep their current value. An empty
/// `default_assignee_worker_id` clears the assignment; a present
/// `member_worker_ids` replaces the ordered membership (add/remove/reorder).
#[derive(Debug, Deserialize)]
pub(super) struct UpdateGroupRequest {
    pub(super) title: Option<String>,
    pub(super) execution_mode: Option<HiveGroupExecutionMode>,
    pub(super) max_rounds: Option<u32>,
    pub(super) max_member_messages_per_turn: Option<u32>,
    pub(super) parallelism: Option<u32>,
    pub(super) context_window_messages: Option<u32>,
    pub(super) default_assignee_worker_id: Option<String>,
    pub(super) member_worker_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SendGroupMessageRequest {
    pub(super) message: String,
    #[serde(default)]
    pub(super) mentions_override: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub(super) struct SendGroupMessageResponse {
    pub(super) group_id: String,
    pub(super) turn_id: String,
    pub(super) message_id: String,
    pub(super) message_seq: i64,
    pub(super) status: String,
    pub(super) target_worker_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ListGroupMessagesQuery {
    pub(super) after_seq: Option<i64>,
    pub(super) limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub(super) struct HiveGroupMessagesResponse {
    pub(super) messages: Vec<HiveGroupMessage>,
    pub(super) latest_seq: i64,
}

#[derive(Debug, Deserialize)]
pub(super) struct ObserveGroupQuery {
    pub(super) after_seq: Option<i64>,
}

pub(super) async fn list_groups(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<HiveGroupsResponse>, AppError> {
    let store = open_group_store(&state)?;
    let groups = store.list_for_owner(current_user_id(user.as_ref()), false)?;
    let mut summaries = Vec::with_capacity(groups.len());
    for group in groups {
        summaries.push(summarize_group(&store, group)?);
    }
    Ok(Json(HiveGroupsResponse { groups: summaries }))
}

pub(super) async fn create_group(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<CreateGroupRequest>,
) -> Result<(StatusCode, Json<HiveGroupDetailResponse>), AppError> {
    let user_id = current_user_id(user.as_ref());
    if req.member_worker_ids.is_empty() {
        return Err(AppError::BadRequest(
            "pick at least one member Worker".to_string(),
        ));
    }
    let store = open_group_store(&state)?;
    let group = store
        .create(&NewHiveGroup {
            user_id: user_id.map(ToOwned::to_owned),
            title: req.title,
            execution_mode: req.execution_mode.unwrap_or_default(),
            max_rounds: req.max_rounds,
            max_member_messages_per_turn: req.max_member_messages_per_turn,
            parallelism: req.parallelism,
            context_window_messages: req.context_window_messages,
            default_assignee_worker_id: trimmed_nonempty(req.default_assignee_worker_id.as_deref())
                .map(ToOwned::to_owned),
            member_worker_ids: req.member_worker_ids,
        })
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    let detail = load_group_detail(&store, group)?;
    Ok((StatusCode::CREATED, Json(detail)))
}

pub(super) async fn get_group(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<HiveGroupDetailResponse>, AppError> {
    let store = open_group_store(&state)?;
    let group = load_owned_group(&store, &id, user.as_ref())?;
    Ok(Json(load_group_detail(&store, group)?))
}

pub(super) async fn update_group(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    Json(req): Json<UpdateGroupRequest>,
) -> Result<Json<HiveGroupDetailResponse>, AppError> {
    let store = open_group_store(&state)?;
    let group = load_owned_group(&store, &id, user.as_ref())?;
    if group.status == HiveGroupStatus::Archived {
        return Err(AppError::Conflict(
            "Cannot update an archived group".to_string(),
        ));
    }

    // Membership first so a same-request assignee change validates against
    // the new roster.
    if let Some(member_worker_ids) = req.member_worker_ids.as_ref() {
        store
            .set_members(&group.id, member_worker_ids)
            .map_err(|error| AppError::BadRequest(error.to_string()))?;
    }
    let group = store.get(&group.id)?.ok_or_else(|| group_not_found(&id))?;

    let title = match req.title.as_deref() {
        Some(value) => trimmed_nonempty(Some(value))
            .ok_or_else(|| AppError::BadRequest("title must not be empty".to_string()))?
            .to_string(),
        None => group.title.clone(),
    };
    let default_assignee = match req.default_assignee_worker_id.as_deref() {
        // An explicitly empty value clears the assignment.
        Some(value) => trimmed_nonempty(Some(value)).map(ToOwned::to_owned),
        None => group.default_assignee_worker_id.clone(),
    };
    let update = HiveGroupUpdate {
        title,
        execution_mode: req.execution_mode.unwrap_or(group.execution_mode),
        max_rounds: req.max_rounds.unwrap_or(group.max_rounds),
        max_member_messages_per_turn: req
            .max_member_messages_per_turn
            .unwrap_or(group.max_member_messages_per_turn),
        parallelism: req.parallelism.unwrap_or(group.parallelism),
        context_window_messages: req
            .context_window_messages
            .unwrap_or(group.context_window_messages),
        default_assignee_worker_id: default_assignee,
    };
    let updated = store
        .update_settings(&group.id, &update)
        .map_err(|error| AppError::BadRequest(error.to_string()))?
        .ok_or_else(|| group_not_found(&id))?;
    Ok(Json(load_group_detail(&store, updated)?))
}

/// Archive (never hard-delete): the timeline survives and member Workers are
/// untouched. Returns a JSON body because the shared client decodes every
/// successful response.
pub(super) async fn archive_group(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<OkResponse>, AppError> {
    let store = open_group_store(&state)?;
    let group = load_owned_group(&store, &id, user.as_ref())?;
    if group.status != HiveGroupStatus::Archived {
        store.set_status(&group.id, HiveGroupStatus::Archived)?;
    }
    Ok(Json(OkResponse { ok: true }))
}

pub(super) async fn send_group_message(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<SendGroupMessageRequest>,
) -> Result<(StatusCode, Json<SendGroupMessageResponse>), AppError> {
    let user_id = current_user_id(user.as_ref());
    let store = open_group_store(&state)?;
    let group = load_owned_group(&store, &id, user.as_ref())?;
    if group.status == HiveGroupStatus::Archived {
        return Err(AppError::Conflict(
            "Cannot message an archived group".to_string(),
        ));
    }
    let message = req.message.trim();
    if message.is_empty() {
        return Err(AppError::BadRequest(
            "message must not be empty".to_string(),
        ));
    }
    if message.len() > MAX_HIVE_GROUP_MESSAGE_BYTES {
        return Err(AppError::BadRequest(format!(
            "message exceeds {MAX_HIVE_GROUP_MESSAGE_BYTES} bytes"
        )));
    }

    // Members picked in the editor may never have been opened one-on-one;
    // ensure each has its DM lane so the fan-out has a controller to queue
    // on. Workspace/model prerequisites stay per-member turn outcomes.
    ensure_member_dm_sessions(&state, user_id, &store, &group)?;

    let idempotency_key = idempotency_key_from_headers(&headers)?;
    let turn = state
        .hive_runtime
        .group_message_for_user(
            &group.id,
            message,
            req.mentions_override,
            user_id,
            idempotency_key.as_deref(),
        )
        .await
        .map_err(super::sessions::hive_control_error)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(SendGroupMessageResponse {
            group_id: turn.group_id,
            turn_id: turn.turn_id,
            message_id: turn.message_id,
            message_seq: turn.message_seq,
            status: turn.status,
            target_worker_ids: turn.target_worker_ids,
        }),
    ))
}

pub(super) async fn list_group_messages(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    Query(query): Query<ListGroupMessagesQuery>,
) -> Result<Json<HiveGroupMessagesResponse>, AppError> {
    let store = open_group_store(&state)?;
    let group = load_owned_group(&store, &id, user.as_ref())?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_MESSAGE_PAGE)
        .clamp(1, MAX_MESSAGE_PAGE);
    let messages = match query.after_seq {
        Some(after_seq) => store.list_messages_after(&group.id, after_seq, limit)?,
        None => store.list_recent_messages(&group.id, limit)?,
    };
    Ok(Json(HiveGroupMessagesResponse {
        latest_seq: store.latest_seq(&group.id)?,
        messages,
    }))
}

pub(super) async fn get_group_turn(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path((id, turn_id)): Path<(String, String)>,
) -> Result<Json<HiveGroupTurnView>, AppError> {
    let store = open_group_store(&state)?;
    let group = load_owned_group(&store, &id, user.as_ref())?;
    let turn = store
        .get_turn(&turn_id)?
        .filter(|turn| turn.group_id == group.id)
        .ok_or_else(|| AppError::NotFound(format!("Turn {turn_id} not found")))?;
    Ok(Json(turn_view(turn)))
}

pub(super) async fn stop_group(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<OkResponse>, AppError> {
    let user_id = current_user_id(user.as_ref());
    let store = open_group_store(&state)?;
    let group = load_owned_group(&store, &id, user.as_ref())?;
    let idempotency_key = idempotency_key_from_headers(&headers)?;
    state
        .hive_runtime
        .group_stop_for_user(&group.id, user_id, idempotency_key.as_deref())
        .await
        .map_err(super::sessions::hive_control_error)?;
    Ok(Json(OkResponse { ok: true }))
}

/// Server-sent room events: message appends plus turn/member status
/// transitions. Implemented as a per-client DB tail with an `after_seq`
/// cursor — the bounded channel gives natural per-client backpressure, and
/// because the cursor replays from durable rows a slow client loses nothing
/// and can never stall the scheduler.
pub(super) async fn observe_group(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    Query(query): Query<ObserveGroupQuery>,
) -> Result<Sse<ReceiverStream<std::result::Result<Event, Infallible>>>, AppError> {
    let store = open_group_store(&state)?;
    let group = load_owned_group(&store, &id, user.as_ref())?;
    let mut after_seq = match query.after_seq {
        Some(after_seq) => after_seq.max(0),
        // Live-only by default: begin after the current high water mark.
        None => store.latest_seq(&group.id)?,
    };

    let (tx, rx) =
        mpsc::channel::<std::result::Result<Event, Infallible>>(GROUP_EVENT_STREAM_BUFFER);
    let db_path = state.db_path;
    let group_id = group.id;
    tokio::spawn(async move {
        let mut last_turn_fingerprint = String::new();
        loop {
            let Ok(db) = Database::new(db_path.as_ref()) else {
                break;
            };
            let store = HiveGroupStore::new(db);

            let messages = match store.list_messages_after(&group_id, after_seq, 200) {
                Ok(messages) => messages,
                Err(_) => break,
            };
            for message in messages {
                after_seq = after_seq.max(message.seq);
                let Ok(event) = Event::default()
                    .json_data(serde_json::json!({"type": "message", "message": message}))
                else {
                    continue;
                };
                if tx.send(Ok(event)).await.is_err() {
                    return;
                }
            }

            // Turn/member status transitions: emit the newest turn whenever
            // its observable state changes.
            let turn = store
                .active_turn(&group_id)
                .ok()
                .flatten()
                .map(Some)
                .unwrap_or_else(|| {
                    store
                        .list_turns(&group_id, 1)
                        .ok()
                        .and_then(|turns| turns.into_iter().next())
                });
            if let Some(turn) = turn {
                let fingerprint = format!(
                    "{}:{}:{}:{}",
                    turn.id, turn.status, turn.next_speaker_index, turn.updated_at
                );
                if fingerprint != last_turn_fingerprint {
                    last_turn_fingerprint = fingerprint;
                    let Ok(event) = Event::default()
                        .json_data(serde_json::json!({"type": "turn", "turn": turn_view(turn)}))
                    else {
                        continue;
                    };
                    if tx.send(Ok(event)).await.is_err() {
                        return;
                    }
                }
            }

            tokio::time::sleep(GROUP_EVENT_POLL).await;
        }
    });

    Ok(Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default()))
}

fn open_group_store(state: &AppState) -> Result<HiveGroupStore, AppError> {
    Ok(HiveGroupStore::new(Database::new(&state.db_path)?))
}

fn open_worker_store(state: &AppState) -> Result<HiveWorkerStore, AppError> {
    Ok(HiveWorkerStore::new(Database::new(&state.db_path)?))
}

/// Exact-owner load: a group owned by another user (or by the local NULL
/// profile) is indistinguishable from a missing one.
fn load_owned_group(
    store: &HiveGroupStore,
    id: &str,
    user: Option<&CurrentUser>,
) -> Result<HiveGroup, AppError> {
    let group = store.get(id)?.ok_or_else(|| group_not_found(id))?;
    if group.user_id.as_deref() != current_user_id(user) {
        return Err(group_not_found(id));
    }
    Ok(group)
}

fn group_not_found(id: &str) -> AppError {
    AppError::NotFound(format!("Group {id} not found"))
}

fn summarize_group(store: &HiveGroupStore, group: HiveGroup) -> Result<HiveGroupSummary, AppError> {
    let members = store
        .member_workers(&group.id)?
        .into_iter()
        .map(member_summary)
        .collect();
    let active_turn_id = store.active_turn(&group.id)?.map(|turn| turn.id);
    let latest_seq = store.latest_seq(&group.id)?;
    Ok(HiveGroupSummary {
        id: group.id,
        title: group.title,
        execution_mode: group.execution_mode.as_str().to_string(),
        max_rounds: group.max_rounds,
        max_member_messages_per_turn: group.max_member_messages_per_turn,
        parallelism: group.parallelism,
        context_window_messages: group.context_window_messages,
        status: group.status.as_str().to_string(),
        default_assignee_worker_id: group.default_assignee_worker_id,
        members,
        active_turn_id,
        latest_seq,
        created_at: group.created_at,
        updated_at: group.updated_at,
    })
}

fn member_summary(worker: HiveWorker) -> HiveGroupMemberSummary {
    HiveGroupMemberSummary {
        provider: worker
            .model_key
            .as_ref()
            .map(|key| key.provider.to_string()),
        worker_id: worker.id,
        slug: worker.slug,
        display_name: worker.display_name,
        avatar_color: worker.avatar_color,
        model: worker.model,
        status: worker.status.as_str().to_string(),
    }
}

fn load_group_detail(
    store: &HiveGroupStore,
    group: HiveGroup,
) -> Result<HiveGroupDetailResponse, AppError> {
    let active_turn = store.active_turn(&group.id)?.map(turn_view);
    Ok(HiveGroupDetailResponse {
        group: summarize_group(store, group)?,
        active_turn,
    })
}

fn turn_view(turn: HiveGroupTurn) -> HiveGroupTurnView {
    HiveGroupTurnView {
        id: turn.id,
        group_id: turn.group_id,
        trigger_message_id: turn.trigger_message_id,
        execution_mode: turn.execution_mode.as_str().to_string(),
        status: turn.status.as_str().to_string(),
        speaker_plan: turn.speaker_plan,
        next_speaker_index: turn.next_speaker_index,
        member_outcomes: turn.member_outcomes,
        started_at: turn.started_at,
        finished_at: turn.finished_at,
    }
}

/// Bind a DM session for every member that lacks one, mirroring the
/// Worker DM ensure route (parentless, workspace-neutral, frozen to the
/// Worker's model identity).
fn ensure_member_dm_sessions(
    state: &AppState,
    user_id: Option<&str>,
    store: &HiveGroupStore,
    group: &HiveGroup,
) -> Result<(), AppError> {
    let members = store.member_workers(&group.id)?;
    let needs_dm = members.iter().any(|worker| {
        worker.dm_session_id.is_none() && worker.status != HiveWorkerStatus::Archived
    });
    if !needs_dm {
        return Ok(());
    }
    let session_manager = open_session_manager(state)?;
    let worker_store = open_worker_store(state)?;
    for worker in members {
        if worker.dm_session_id.is_some() || worker.status == HiveWorkerStatus::Archived {
            continue;
        }
        let session_id = session_manager.create_session_for_user_with_config_and_permission(
            &worker.display_name,
            worker.model.as_deref(),
            None,
            None,
            WorkspaceMode::Neutral,
            user_id,
            None,
            SessionType::Hive,
            worker.permission_mode,
        )?;
        if let Some(key) = worker.model_key.as_ref() {
            session_manager.update_session_model_selection(
                &session_id,
                Some(key),
                worker.model_catalog_revision.as_deref(),
            )?;
        }
        worker_store.bind_dm_session(&worker.id, Some(&session_id))?;
    }
    Ok(())
}
