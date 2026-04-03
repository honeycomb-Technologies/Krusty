//! Report/Paper endpoints

use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use krusty_core::storage::{Database, ReportStore};

use crate::error::AppError;
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct ListReportsQuery {
    pub project_dir: Option<String>,
}

#[derive(Serialize)]
pub struct ReportSummaryResponse {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub tags: Vec<String>,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct ReportDetailResponse {
    pub id: String,
    pub title: String,
    pub content: String,
    pub summary: String,
    pub tags: Vec<String>,
    pub sources: Vec<String>,
    pub session_id: String,
    pub project_dir: Option<String>,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct ListReportsResponse {
    pub reports: Vec<ReportSummaryResponse>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_reports))
        .route("/:id", get(get_report))
}

async fn list_reports(
    State(state): State<AppState>,
    Query(query): Query<ListReportsQuery>,
) -> Result<Json<ListReportsResponse>, AppError> {
    let store = ReportStore::new(Database::new(&state.db_path)?);
    let reports = store.list_reports(query.project_dir.as_deref())?;

    let summaries = reports
        .into_iter()
        .map(|r| ReportSummaryResponse {
            id: r.id,
            title: r.title,
            summary: r.summary,
            tags: r.tags,
            created_at: r.created_at,
        })
        .collect();

    Ok(Json(ListReportsResponse { reports: summaries }))
}

async fn get_report(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ReportDetailResponse>, AppError> {
    let store = ReportStore::new(Database::new(&state.db_path)?);
    let report = store
        .get_report(&id)?
        .ok_or_else(|| AppError::NotFound(format!("Report {} not found", id)))?;

    Ok(Json(ReportDetailResponse {
        id: report.id,
        title: report.title,
        content: report.content,
        summary: report.summary,
        tags: report.tags,
        sources: report.sources,
        session_id: report.session_id,
        project_dir: report.project_dir,
        created_at: report.created_at,
    }))
}
