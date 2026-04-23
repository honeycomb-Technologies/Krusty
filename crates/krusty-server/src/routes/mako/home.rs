use std::collections::BTreeMap;

use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};

use krusty_core::storage::{
    bootstrap_mako_home, is_valid_crew_slug, summarize_channel_bindings, summarize_crew_runtime,
    write_mako_crew_document, write_mako_home_document, ApnsDeviceStore, AutonomousTaskStore,
    Database, DelegatedRunStore, MakoChannelBinding, MakoChannelKind, MakoCrewDocumentKind,
    MakoCrewRuntimeSummary, MakoHomeDocument, MakoHomeDocumentKind, MakoHomeProfile,
    MakoRuntimeState, SessionInfo, SessionType,
};

use super::super::session_access::current_user_id;
use super::{mako_home_dir_for_user, open_session_manager};
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::utils::text::trimmed_nonempty;
use crate::AppState;

const MAKO_HOME_PREVIEW_CHARS: usize = 160;

#[derive(Debug, Deserialize)]
pub(super) struct DocumentWriteRequest {
    pub(super) content: String,
}

#[derive(Debug, Serialize)]
pub(super) struct MakoHomeDocumentSummary {
    pub(super) file_name: String,
    pub(super) content: String,
    pub(super) preview: String,
}

#[derive(Debug, Serialize)]
pub(super) struct MakoCrewMemberSummary {
    pub(super) slug: String,
    pub(super) identity: Option<MakoHomeDocumentSummary>,
    pub(super) soul: Option<MakoHomeDocumentSummary>,
    pub(super) memory: Option<MakoHomeDocumentSummary>,
}

#[derive(Debug, Serialize)]
pub(super) struct MakoChannelSummary {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) kind: String,
    pub(super) source: String,
    pub(super) enabled: bool,
    pub(super) status: String,
    pub(super) detail: String,
}

#[derive(Debug, Serialize)]
pub(super) struct MakoChannelsResponse {
    pub(super) items: Vec<MakoChannelSummary>,
    pub(super) apns_configured: bool,
    pub(super) apns_device_count: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct MakoHomeResponse {
    pub(super) soul: Option<MakoHomeDocumentSummary>,
    pub(super) identity: Option<MakoHomeDocumentSummary>,
    pub(super) heartbeat: Option<MakoHomeDocumentSummary>,
    pub(super) memory: Option<MakoHomeDocumentSummary>,
    pub(super) channels: Option<MakoHomeDocumentSummary>,
    pub(super) crew: Vec<MakoCrewMemberSummary>,
    pub(super) crew_count: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct MakoBootstrapResponse {
    pub(super) ok: bool,
    pub(super) created_files: Vec<String>,
    pub(super) home: MakoHomeResponse,
}

#[derive(Debug, Serialize)]
pub(super) struct MakoCrewResponse {
    pub(super) members: Vec<MakoCrewRuntimeMemberSummary>,
}

#[derive(Debug, Serialize)]
pub(super) struct MakoCrewRuntimeMemberSummary {
    pub(super) slug: String,
    pub(super) known_to_home: bool,
    pub(super) status: String,
    pub(super) active_run_count: usize,
    pub(super) recent_run_count: usize,
    pub(super) failed_run_count: usize,
    pub(super) queued_task_count: usize,
    pub(super) active_task_count: usize,
    pub(super) completed_task_count: usize,
    pub(super) failed_task_count: usize,
    pub(super) latest_activity_at: Option<String>,
    pub(super) identity: Option<MakoHomeDocumentSummary>,
    pub(super) soul: Option<MakoHomeDocumentSummary>,
    pub(super) memory: Option<MakoHomeDocumentSummary>,
}

pub(super) async fn home(
    State(_state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<MakoHomeResponse>, AppError> {
    Ok(Json(build_mako_home_response_from_dir(
        &mako_home_dir_for_user(user.as_ref()),
    )))
}

pub(super) async fn bootstrap_home(
    State(_state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<MakoBootstrapResponse>, AppError> {
    Ok(Json(build_mako_bootstrap_response_from_dir(
        &mako_home_dir_for_user(user.as_ref()),
    )?))
}

pub(super) async fn update_home_document(
    State(_state): State<AppState>,
    user: Option<CurrentUser>,
    Path(kind): Path<String>,
    Json(req): Json<DocumentWriteRequest>,
) -> Result<Json<MakoHomeResponse>, AppError> {
    let kind = MakoHomeDocumentKind::parse(&kind)
        .ok_or_else(|| AppError::BadRequest("invalid Mako home document kind".to_string()))?;
    let content = trimmed_nonempty(Some(req.content.as_str()))
        .ok_or_else(|| AppError::BadRequest("content must not be empty".to_string()))?;
    let mako_home = mako_home_dir_for_user(user.as_ref());

    write_mako_home_document(&mako_home, kind, content).map_err(|error| {
        AppError::Internal(format!("Failed to update Mako home document: {}", error))
    })?;

    Ok(Json(build_mako_home_response_from_dir(&mako_home)))
}

pub(super) async fn update_crew_document(
    State(_state): State<AppState>,
    user: Option<CurrentUser>,
    Path((slug, kind)): Path<(String, String)>,
    Json(req): Json<DocumentWriteRequest>,
) -> Result<Json<MakoHomeResponse>, AppError> {
    if !is_valid_crew_slug(&slug) {
        return Err(AppError::BadRequest("invalid crew slug".to_string()));
    }
    let kind = MakoCrewDocumentKind::parse(&kind)
        .ok_or_else(|| AppError::BadRequest("invalid Mako crew document kind".to_string()))?;
    let content = trimmed_nonempty(Some(req.content.as_str()))
        .ok_or_else(|| AppError::BadRequest("content must not be empty".to_string()))?;
    let mako_home = mako_home_dir_for_user(user.as_ref());

    write_mako_crew_document(&mako_home, &slug, kind, content).map_err(|error| {
        AppError::Internal(format!("Failed to update Mako crew document: {}", error))
    })?;

    Ok(Json(build_mako_home_response_from_dir(&mako_home)))
}

pub(super) async fn crew(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<MakoCrewResponse>, AppError> {
    let user_id = current_user_id(user.as_ref());
    let (sessions, runtime_states) = {
        let session_manager = open_session_manager(&state)?;
        let sessions =
            session_manager.list_sessions_for_user_by_type(None, user_id, SessionType::Mako)?;
        let runtime_states =
            krusty_core::storage::MakoRuntimeStateStore::new(Database::new(&state.db_path)?)
                .list_states_for_sessions(
                    &sessions
                        .iter()
                        .map(|session| session.id.clone())
                        .collect::<Vec<_>>(),
                )?;
        (sessions, runtime_states)
    };
    let task_store = AutonomousTaskStore::new(Database::new(&state.db_path)?);
    let delegated_store = DelegatedRunStore::new(Database::new(&state.db_path)?);

    Ok(Json(build_mako_crew_response_from_dir_and_sessions(
        &mako_home_dir_for_user(user.as_ref()),
        &sessions,
        &runtime_states,
        &task_store,
        &delegated_store,
    )?))
}

pub(super) async fn channels(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<MakoChannelsResponse>, AppError> {
    let mako_home = mako_home_dir_for_user(user.as_ref());
    let db = Database::new(&state.db_path)?;
    let apns_store = ApnsDeviceStore::new(&db);
    let apns_device_count = apns_store.count_for_user(current_user_id(user.as_ref()))?;

    Ok(Json(build_mako_channels_response_from_dir(
        &state,
        &mako_home,
        apns_device_count,
    )))
}

pub(super) fn build_mako_bootstrap_response_from_dir(
    mako_home: &std::path::Path,
) -> Result<MakoBootstrapResponse, AppError> {
    let result = bootstrap_mako_home(mako_home)
        .map_err(|error| AppError::Internal(format!("Failed to bootstrap Mako home: {}", error)))?;
    Ok(MakoBootstrapResponse {
        ok: true,
        created_files: result.created_files,
        home: build_mako_home_response_from_dir(mako_home),
    })
}

pub(super) fn build_mako_home_response_from_dir(mako_home: &std::path::Path) -> MakoHomeResponse {
    let profile = MakoHomeProfile::load_from(mako_home);

    MakoHomeResponse {
        soul: summarize_mako_home_document(profile.soul),
        identity: summarize_mako_home_document(profile.identity),
        heartbeat: summarize_mako_home_document(profile.heartbeat),
        memory: summarize_mako_home_document(profile.memory),
        channels: summarize_mako_home_document(profile.channels),
        crew_count: profile.crew.len(),
        crew: profile
            .crew
            .into_iter()
            .map(|member| MakoCrewMemberSummary {
                slug: member.slug,
                identity: summarize_mako_home_document(member.identity),
                soul: summarize_mako_home_document(member.soul),
                memory: summarize_mako_home_document(member.memory),
            })
            .collect(),
    }
}

pub(super) fn build_mako_crew_response_from_dir_and_sessions(
    mako_home: &std::path::Path,
    sessions: &[SessionInfo],
    runtime_states: &std::collections::HashMap<String, MakoRuntimeState>,
    task_store: &AutonomousTaskStore,
    delegated_store: &DelegatedRunStore,
) -> Result<MakoCrewResponse, AppError> {
    let profile = MakoHomeProfile::load_from(mako_home);
    let profile_map = profile
        .crew
        .iter()
        .map(|member| (member.slug.as_str(), member))
        .collect::<BTreeMap<_, _>>();
    let runtime = summarize_crew_runtime(
        &profile,
        sessions,
        runtime_states,
        task_store,
        delegated_store,
    )
    .map_err(|error| AppError::Internal(format!("Failed to summarize Mako crew: {}", error)))?;

    Ok(MakoCrewResponse {
        members: runtime
            .into_iter()
            .map(|member| {
                let profile = profile_map.get(member.slug.as_str());
                summarize_mako_crew_member(member, profile.copied())
            })
            .collect(),
    })
}

pub(super) fn build_mako_channels_response_from_dir(
    state: &AppState,
    mako_home: &std::path::Path,
    apns_device_count: usize,
) -> MakoChannelsResponse {
    let profile = MakoHomeProfile::load_from(mako_home);
    let apns_configured = state.apns_service.is_some();

    MakoChannelsResponse {
        items: summarize_channel_bindings(&profile)
            .into_iter()
            .map(|binding| summarize_mako_channel(binding, apns_configured, apns_device_count))
            .collect(),
        apns_configured,
        apns_device_count,
    }
}

fn summarize_mako_crew_member(
    runtime: MakoCrewRuntimeSummary,
    profile: Option<&krusty_core::storage::MakoCrewProfile>,
) -> MakoCrewRuntimeMemberSummary {
    MakoCrewRuntimeMemberSummary {
        slug: runtime.slug,
        known_to_home: runtime.known_to_home,
        status: runtime.status.as_str().to_string(),
        active_run_count: runtime.active_run_count,
        recent_run_count: runtime.recent_run_count,
        failed_run_count: runtime.failed_run_count,
        queued_task_count: runtime.queued_task_count,
        active_task_count: runtime.active_task_count,
        completed_task_count: runtime.completed_task_count,
        failed_task_count: runtime.failed_task_count,
        latest_activity_at: runtime.latest_activity_at,
        identity: summarize_mako_home_document(profile.and_then(|member| member.identity.clone())),
        soul: summarize_mako_home_document(profile.and_then(|member| member.soul.clone())),
        memory: summarize_mako_home_document(profile.and_then(|member| member.memory.clone())),
    }
}

fn summarize_mako_channel(
    binding: MakoChannelBinding,
    apns_configured: bool,
    apns_device_count: usize,
) -> MakoChannelSummary {
    let (status, detail) = match binding.kind {
        MakoChannelKind::MainThread => ("ready", binding.detail),
        MakoChannelKind::Crew => {
            if binding.enabled {
                ("ready", binding.detail)
            } else {
                ("inactive", binding.detail)
            }
        }
        MakoChannelKind::MobilePush => {
            if !binding.enabled {
                ("inactive", binding.detail)
            } else if !apns_configured {
                (
                    "attention",
                    format!("{} APNs is not configured on this server.", binding.detail),
                )
            } else if apns_device_count == 0 {
                (
                    "attention",
                    format!("{} No iPhone devices are registered yet.", binding.detail),
                )
            } else {
                (
                    "ready",
                    format!(
                        "{} {} device{} ready for push delivery.",
                        binding.detail,
                        apns_device_count,
                        if apns_device_count == 1 { "" } else { "s" }
                    ),
                )
            }
        }
        _ => {
            if binding.enabled {
                ("configured", binding.detail)
            } else {
                ("inactive", binding.detail)
            }
        }
    };

    MakoChannelSummary {
        id: binding.id,
        label: binding.label,
        kind: binding.kind.as_str().to_string(),
        source: binding.source.to_string(),
        enabled: binding.enabled,
        status: status.to_string(),
        detail: detail.trim().to_string(),
    }
}

fn summarize_mako_home_document(
    document: Option<MakoHomeDocument>,
) -> Option<MakoHomeDocumentSummary> {
    document.map(|document| MakoHomeDocumentSummary {
        preview: truncate_preview(&document.content, MAKO_HOME_PREVIEW_CHARS),
        file_name: document.file_name,
        content: document.content,
    })
}

fn truncate_preview(value: &str, max_chars: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max_chars {
        return value.to_string();
    }
    let truncated = value.chars().take(max_chars).collect::<String>();
    format!("{}...", truncated)
}
