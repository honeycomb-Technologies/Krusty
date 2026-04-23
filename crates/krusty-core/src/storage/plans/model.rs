use chrono::{DateTime, Utc};
use rusqlite::Row;

use crate::plan::{PlanFile, PlanStatus};

pub(super) struct PlanRow {
    pub title: String,
    pub status: String,
    pub content: String,
    pub created_at: String,
}

impl PlanRow {
    pub(super) fn into_plan_file(self, session_id: &str) -> anyhow::Result<PlanFile> {
        let mut plan = PlanFile::from_markdown(&self.content)
            .map_err(|e| anyhow::anyhow!("Failed to parse plan: {}", e))?;

        plan.title = self.title;
        plan.session_id = Some(session_id.to_string());

        if let Ok(status) = self.status.parse::<PlanStatus>() {
            plan.status = status;
        }

        if let Ok(dt) = DateTime::parse_from_rfc3339(&self.created_at) {
            plan.created_at = dt.with_timezone(&Utc);
        }

        Ok(plan)
    }
}

#[derive(Debug, Clone)]
pub struct PlanSummary {
    pub id: String,
    pub session_id: String,
    pub title: String,
    pub status: PlanStatus,
    pub created_at: String,
    pub working_dir: Option<String>,
}

pub(super) fn plan_summary_from_row(row: &Row<'_>) -> rusqlite::Result<PlanSummary> {
    Ok(PlanSummary {
        id: row.get(0)?,
        session_id: row.get(1)?,
        title: row.get(2)?,
        status: row
            .get::<_, String>(3)?
            .parse()
            .unwrap_or(PlanStatus::InProgress),
        created_at: row.get(4)?,
        working_dir: row.get(5)?,
    })
}
