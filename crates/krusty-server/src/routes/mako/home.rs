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
    MakoCrewProfileDocumentKind, MakoProfileDocument, MakoProfileDocumentKind, MakoProfileOwner,
    MakoProfileSnapshot, MakoProfileStore, MakoProfileStoreError, MakoRuntimeState, SessionInfo,
    SessionType,
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
    #[serde(default)]
    pub(super) expected_revision: Option<i64>,
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
    pub(super) profile_id: Option<String>,
    pub(super) revision: Option<i64>,
    pub(super) soul: Option<MakoHomeDocumentSummary>,
    pub(super) identity: Option<MakoHomeDocumentSummary>,
    pub(super) user: Option<MakoHomeDocumentSummary>,
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
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<MakoHomeResponse>, AppError> {
    let profile = load_or_bootstrap_profile(&state, user.as_ref())?;
    Ok(Json(build_mako_home_response_from_profile(&profile)))
}

pub(super) async fn bootstrap_home(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<MakoBootstrapResponse>, AppError> {
    let owner = profile_owner(user.as_ref())?;
    let store = MakoProfileStore::new(Database::new(&state.db_path)?);
    if owner.is_local() {
        store
            .import_local_legacy_home(&owner, &krusty_core::paths::mako_dir())
            .map_err(map_profile_error)?;
    }
    let merged = store
        .bootstrap_defaults(&owner)
        .map_err(map_profile_error)?;
    let mut created_files = merged
        .inserted_documents
        .iter()
        .map(|kind| kind.preferred_file_name().to_string())
        .collect::<Vec<_>>();
    created_files.extend(merged.inserted_crew_documents.iter().map(|(slug, kind)| {
        format!("crew/{slug}/{}", kind.preferred_file_name())
    }));
    Ok(Json(MakoBootstrapResponse {
        ok: true,
        created_files,
        home: build_mako_home_response_from_profile(&merged.snapshot),
    }))
}

pub(super) async fn update_home_document(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(kind): Path<String>,
    Json(req): Json<DocumentWriteRequest>,
) -> Result<Json<MakoHomeResponse>, AppError> {
    let kind = MakoProfileDocumentKind::parse(&kind).ok_or_else(|| {
        AppError::BadRequest(
            "invalid Mako profile document kind; memory is managed by the memory API".to_string(),
        )
    })?;
    let content = trimmed_nonempty(Some(req.content.as_str()))
        .ok_or_else(|| AppError::BadRequest("content must not be empty".to_string()))?;
    let owner = profile_owner(user.as_ref())?;
    let store = MakoProfileStore::new(Database::new(&state.db_path)?);
    let snapshot = bootstrap_profile(&store, &owner)?;
    let updated = store
        .update_document(
            &owner,
            kind,
            content,
            req.expected_revision.unwrap_or(snapshot.revision),
        )
        .map_err(map_profile_error)?;

    Ok(Json(build_mako_home_response_from_profile(&updated)))
}

pub(super) async fn update_crew_document(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path((slug, kind)): Path<(String, String)>,
    Json(req): Json<DocumentWriteRequest>,
) -> Result<Json<MakoHomeResponse>, AppError> {
    if !is_valid_crew_slug(&slug) {
        return Err(AppError::BadRequest("invalid crew slug".to_string()));
    }
    let kind = MakoCrewProfileDocumentKind::parse(&kind).ok_or_else(|| {
        AppError::BadRequest(
            "invalid Mako crew profile kind; crew memory is canonical memory, not identity"
                .to_string(),
        )
    })?;
    let content = trimmed_nonempty(Some(req.content.as_str()))
        .ok_or_else(|| AppError::BadRequest("content must not be empty".to_string()))?;
    let owner = profile_owner(user.as_ref())?;
    let store = MakoProfileStore::new(Database::new(&state.db_path)?);
    let snapshot = bootstrap_profile(&store, &owner)?;
    let updated = store
        .update_crew_document(
            &owner,
            &slug,
            kind,
            content,
            req.expected_revision.unwrap_or(snapshot.revision),
        )
        .map_err(map_profile_error)?;

    Ok(Json(build_mako_home_response_from_profile(&updated)))
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

    let profile = load_or_bootstrap_profile(&state, user.as_ref())?;
    Ok(Json(build_mako_crew_response_from_profile_and_sessions(
        &profile,
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
    let profile = load_or_bootstrap_profile(&state, user.as_ref())?;
    let db = Database::new(&state.db_path)?;
    let apns_store = ApnsDeviceStore::new(&db);
    let apns_device_count = apns_store.count_for_user(current_user_id(user.as_ref()))?;

    Ok(Json(build_mako_channels_response_from_profile(
        &state,
        &profile,
        apns_device_count,
    )))
}

fn profile_owner(user: Option<&CurrentUser>) -> Result<MakoProfileOwner, AppError> {
    MakoProfileOwner::from_user_id(current_user_id(user))
        .map_err(|error| AppError::BadRequest(error.to_string()))
}

fn bootstrap_profile(
    store: &MakoProfileStore,
    owner: &MakoProfileOwner,
) -> Result<MakoProfileSnapshot, AppError> {
    if owner.is_local() {
        store
            .import_local_legacy_home(owner, &krusty_core::paths::mako_dir())
            .map_err(map_profile_error)?;
    }
    store
        .bootstrap_defaults(owner)
        .map(|result| result.snapshot)
        .map_err(map_profile_error)
}

fn load_or_bootstrap_profile(
    state: &AppState,
    user: Option<&CurrentUser>,
) -> Result<MakoProfileSnapshot, AppError> {
    let owner = profile_owner(user)?;
    let store = MakoProfileStore::new(Database::new(&state.db_path)?);
    bootstrap_profile(&store, &owner)
}

fn map_profile_error(error: MakoProfileStoreError) -> AppError {
    let message = error.to_string();
    match error {
        MakoProfileStoreError::RevisionConflict { .. } => AppError::Conflict(message),
        MakoProfileStoreError::EmptyContent
        | MakoProfileStoreError::InvalidCrewSlug(_)
        | MakoProfileStoreError::InvalidOwner(_) => AppError::BadRequest(message),
        _ => AppError::Internal(message),
    }
}

fn summarize_profile_document<K>(
    document: Option<MakoProfileDocument<K>>,
    file_name: &str,
) -> Option<MakoHomeDocumentSummary> {
    document.map(|document| MakoHomeDocumentSummary {
        preview: truncate_preview(&document.content, MAKO_HOME_PREVIEW_CHARS),
        file_name: file_name.to_string(),
        content: document.content,
    })
}

fn build_mako_home_response_from_profile(profile: &MakoProfileSnapshot) -> MakoHomeResponse {
    MakoHomeResponse {
        profile_id: Some(profile.profile_id.clone()),
        revision: Some(profile.revision),
        soul: summarize_profile_document(
            profile.soul.clone(),
            MakoProfileDocumentKind::Soul.preferred_file_name(),
        ),
        identity: summarize_profile_document(
            profile.identity.clone(),
            MakoProfileDocumentKind::Identity.preferred_file_name(),
        ),
        user: summarize_profile_document(
            profile.user.clone(),
            MakoProfileDocumentKind::User.preferred_file_name(),
        ),
        heartbeat: summarize_profile_document(
            profile.heartbeat.clone(),
            MakoProfileDocumentKind::Heartbeat.preferred_file_name(),
        ),
        memory: None,
        channels: summarize_profile_document(
            profile.channels.clone(),
            MakoProfileDocumentKind::Channels.preferred_file_name(),
        ),
        crew_count: profile.crew.len(),
        crew: profile
            .crew
            .iter()
            .map(|member| MakoCrewMemberSummary {
                slug: member.slug.clone(),
                identity: summarize_profile_document(
                    member.identity.clone(),
                    MakoCrewProfileDocumentKind::Identity.preferred_file_name(),
                ),
                soul: summarize_profile_document(
                    member.soul.clone(),
                    MakoCrewProfileDocumentKind::Soul.preferred_file_name(),
                ),
                memory: None,
            })
            .collect(),
    }
}

fn legacy_profile_from_snapshot(profile: &MakoProfileSnapshot) -> MakoHomeProfile {
    let document = |value: &Option<MakoProfileDocument<MakoProfileDocumentKind>>, file_name: &str| {
        value.as_ref().map(|document| MakoHomeDocument {
            file_name: file_name.to_string(),
            content: document.content.clone(),
        })
    };
    MakoHomeProfile {
        soul: document(&profile.soul, krusty_core::paths::MAKO_SOUL_FILE),
        identity: document(&profile.identity, krusty_core::paths::MAKO_IDENTITY_FILE),
        user: document(&profile.user, krusty_core::paths::MAKO_USER_FILE),
        heartbeat: document(&profile.heartbeat, krusty_core::paths::MAKO_HEARTBEAT_FILE),
        memory: None,
        channels: document(&profile.channels, krusty_core::paths::MAKO_CHANNELS_FILE),
        crew: profile
            .crew
            .iter()
            .map(|member| krusty_core::storage::MakoCrewProfile {
                slug: member.slug.clone(),
                identity: member.identity.as_ref().map(|document| MakoHomeDocument {
                    file_name: "IDENTITY.md".to_string(),
                    content: document.content.clone(),
                }),
                soul: member.soul.as_ref().map(|document| MakoHomeDocument {
                    file_name: "SOUL.md".to_string(),
                    content: document.content.clone(),
                }),
                memory: None,
            })
            .collect(),
    }
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
        profile_id: None,
        revision: None,
        soul: summarize_mako_home_document(profile.soul),
        identity: summarize_mako_home_document(profile.identity),
        user: summarize_mako_home_document(profile.user),
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
    build_mako_crew_response_from_loaded_profile(
        &profile,
        sessions,
        runtime_states,
        task_store,
        delegated_store,
    )
}

fn build_mako_crew_response_from_profile_and_sessions(
    profile: &MakoProfileSnapshot,
    sessions: &[SessionInfo],
    runtime_states: &std::collections::HashMap<String, MakoRuntimeState>,
    task_store: &AutonomousTaskStore,
    delegated_store: &DelegatedRunStore,
) -> Result<MakoCrewResponse, AppError> {
    build_mako_crew_response_from_loaded_profile(
        &legacy_profile_from_snapshot(profile),
        sessions,
        runtime_states,
        task_store,
        delegated_store,
    )
}

fn build_mako_crew_response_from_loaded_profile(
    profile: &MakoHomeProfile,
    sessions: &[SessionInfo],
    runtime_states: &std::collections::HashMap<String, MakoRuntimeState>,
    task_store: &AutonomousTaskStore,
    delegated_store: &DelegatedRunStore,
) -> Result<MakoCrewResponse, AppError> {
    let profile_map = profile
        .crew
        .iter()
        .map(|member| (member.slug.as_str(), member))
        .collect::<BTreeMap<_, _>>();
    let runtime = summarize_crew_runtime(
        profile,
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
    build_mako_channels_response_from_loaded_profile(state, &profile, apns_device_count)
}

fn build_mako_channels_response_from_profile(
    state: &AppState,
    profile: &MakoProfileSnapshot,
    apns_device_count: usize,
) -> MakoChannelsResponse {
    build_mako_channels_response_from_loaded_profile(
        state,
        &legacy_profile_from_snapshot(profile),
        apns_device_count,
    )
}

fn build_mako_channels_response_from_loaded_profile(
    state: &AppState,
    profile: &MakoHomeProfile,
    apns_device_count: usize,
) -> MakoChannelsResponse {
    let apns_configured = state.apns_service.is_some();

    MakoChannelsResponse {
        items: summarize_channel_bindings(profile)
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
