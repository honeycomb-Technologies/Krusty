use chrono::{DateTime, Utc};
use std::path::PathBuf;

use crate::plan::PlanStatus;

/// Summary of a plan from database.
#[derive(Debug, Clone)]
pub struct PlanSummary {
    pub id: String,
    pub session_id: Option<String>,
    pub title: String,
    pub status: PlanStatus,
    pub progress: (usize, usize),
    pub created_at: DateTime<Utc>,
    pub working_dir: Option<String>,
}

/// Summary of a legacy file-based plan (for migration).
#[derive(Debug, Clone)]
pub struct LegacyPlanSummary {
    pub path: PathBuf,
    pub title: String,
    pub status: PlanStatus,
    pub progress: (usize, usize),
    pub created_at: DateTime<Utc>,
    pub session_id: Option<String>,
    pub working_dir: Option<String>,
}
