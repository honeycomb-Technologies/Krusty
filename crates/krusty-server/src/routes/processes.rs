//! Process management endpoints

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;

use krusty_core::process::{ProcessInfo, ProcessStatus};

use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::AppState;

/// Build the processes router
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_processes))
        .route("/:id", get(get_process))
        .route("/:id/output", get(get_process_output))
        .route("/:id/kill", post(kill_process))
        .route("/:id/suspend", post(suspend_process))
        .route("/:id/resume", post(resume_process))
}

/// Process info for API response
#[derive(Serialize)]
pub struct ProcessResponse {
    pub id: String,
    pub command: String,
    pub description: Option<String>,
    pub pid: Option<u32>,
    /// Legacy debug-formatted status retained for API compatibility.
    pub status: String,
    /// Stable lowercase status shared with the agent process tool.
    pub status_code: String,
    pub elapsed_secs: u64,
    pub error: Option<String>,
    pub exit_code: Option<i32>,
    pub working_dir: String,
}

#[derive(Serialize)]
pub struct ProcessOutputResponse {
    pub output: String,
    pub truncated: bool,
}

impl From<ProcessInfo> for ProcessResponse {
    fn from(p: ProcessInfo) -> Self {
        let error = match &p.status {
            ProcessStatus::Failed { error, .. } => Some(error.clone()),
            _ => None,
        };
        let exit_code = match &p.status {
            ProcessStatus::Completed { exit_code, .. } => Some(*exit_code),
            _ => None,
        };
        let status = format!("{:?}", p.status);
        let status_code = p.display_status().to_string();
        let elapsed_secs = p.duration().as_secs();
        let working_dir = p._working_dir.to_string_lossy().into_owned();
        Self {
            id: p.id,
            command: p.command,
            description: p.description,
            pid: p.pid,
            status,
            status_code,
            elapsed_secs,
            error,
            exit_code,
            working_dir,
        }
    }
}

/// List all background processes (user-scoped in multi-tenant mode)
async fn list_processes(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Json<Vec<ProcessResponse>> {
    let processes: Vec<ProcessResponse> = match user.and_then(|u| u.0.user_id) {
        Some(user_id) => state.process_registry.list_for_user(&user_id).await,
        None => state.process_registry.list().await,
    }
    .into_iter()
    .map(Into::into)
    .collect();
    Json(processes)
}

/// Get a specific process (user-scoped in multi-tenant mode)
async fn get_process(
    State(state): State<AppState>,
    Path(id): Path<String>,
    user: Option<CurrentUser>,
) -> Result<Json<ProcessResponse>, AppError> {
    let process = match user.and_then(|u| u.0.user_id) {
        Some(user_id) => state.process_registry.get_for_user(&user_id, &id).await,
        None => state.process_registry.get(&id).await,
    }
    .ok_or_else(|| AppError::NotFound(format!("Process {} not found", id)))?;

    Ok(Json(process.into()))
}

/// Read the bounded combined stdout/stderr tail for a tracked process.
async fn get_process_output(
    State(state): State<AppState>,
    Path(id): Path<String>,
    user: Option<CurrentUser>,
) -> Result<Json<ProcessOutputResponse>, AppError> {
    let output = match user.and_then(|u| u.0.user_id) {
        Some(user_id) => state.process_registry.output_for_user(&user_id, &id).await,
        None => state.process_registry.output(&id).await,
    }
    .ok_or_else(|| AppError::NotFound(format!("Process {} not found", id)))?;

    Ok(Json(ProcessOutputResponse {
        output: output.0,
        truncated: output.1,
    }))
}

/// Kill a process (user-scoped in multi-tenant mode)
async fn kill_process(
    State(state): State<AppState>,
    Path(id): Path<String>,
    user: Option<CurrentUser>,
) -> Result<StatusCode, AppError> {
    let result = match user.and_then(|u| u.0.user_id) {
        Some(user_id) => state.process_registry.kill_for_user(&user_id, &id).await,
        None => state.process_registry.kill(&id).await,
    };

    result.map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

/// Suspend a process (user-scoped in multi-tenant mode)
async fn suspend_process(
    State(state): State<AppState>,
    Path(id): Path<String>,
    user: Option<CurrentUser>,
) -> Result<StatusCode, AppError> {
    let result = match user.and_then(|u| u.0.user_id) {
        Some(user_id) => state.process_registry.suspend_for_user(&user_id, &id).await,
        None => state.process_registry.suspend(&id).await,
    };

    result.map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

/// Resume a suspended process (user-scoped in multi-tenant mode)
async fn resume_process(
    State(state): State<AppState>,
    Path(id): Path<String>,
    user: Option<CurrentUser>,
) -> Result<StatusCode, AppError> {
    let result = match user.and_then(|u| u.0.user_id) {
        Some(user_id) => state.process_registry.resume_for_user(&user_id, &id).await,
        None => state.process_registry.resume(&id).await,
    };

    result.map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Instant;

    use krusty_core::process::{ProcessInfo, ProcessStatus};

    use super::ProcessResponse;

    #[test]
    fn process_response_exposes_stable_status_and_failure_details() {
        let response = ProcessResponse::from(ProcessInfo {
            id: "process-1".to_string(),
            command: "preview".to_string(),
            description: Some("Project preview".to_string()),
            pid: Some(42),
            started_at: Instant::now(),
            status: ProcessStatus::Failed {
                error: "address already in use".to_string(),
                duration_ms: 12,
            },
            _working_dir: PathBuf::from("/workspace/project"),
            session_id: None,
            completion_notified: false,
        });

        assert_eq!(response.status_code, "failed");
        assert_eq!(response.error.as_deref(), Some("address already in use"));
        assert_eq!(response.exit_code, None);
        assert_eq!(response.working_dir, "/workspace/project");
    }

    #[test]
    fn completed_process_duration_stops_at_recorded_completion() {
        let response = ProcessResponse::from(ProcessInfo {
            id: "process-2".to_string(),
            command: "build".to_string(),
            description: None,
            pid: Some(43),
            started_at: Instant::now() - std::time::Duration::from_secs(3_600),
            status: ProcessStatus::Completed {
                exit_code: 0,
                duration_ms: 2_500,
            },
            _working_dir: PathBuf::from("/workspace/project"),
            session_id: None,
            completion_notified: false,
        });

        assert_eq!(response.status_code, "done");
        assert_eq!(response.exit_code, Some(0));
        assert_eq!(response.elapsed_secs, 2);
    }
}
