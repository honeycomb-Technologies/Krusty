//! Plan storage with strict session linkage
//!
//! Provides SQLite-backed plan storage with:
//! - 1:1 session-plan relationship (enforced by UNIQUE constraint)
//! - Automatic plan deletion on session delete (CASCADE)
//! - CRUD operations for plans

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Row};

use super::database::Database;
use crate::plan::{PlanFile, PlanStatus};

/// SQLite-backed plan storage
pub struct PlanStore<'a> {
    db: &'a Database,
}

impl<'a> PlanStore<'a> {
    /// Create a new plan store with database reference
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Create or update plan for a session
    ///
    /// If session already has a plan, it will be replaced.
    /// Returns the plan ID.
    pub fn upsert_plan(&self, session_id: &str, plan: &PlanFile) -> Result<String> {
        let now = Utc::now().to_rfc3339();
        let plan_id = uuid::Uuid::new_v4().to_string();
        let content = plan.to_markdown();
        let status = plan.status.to_string();

        // Use INSERT OR REPLACE to handle existing plans
        self.db.conn().execute(
            "INSERT OR REPLACE INTO plans (id, session_id, title, status, content, created_at, updated_at)
             VALUES (
                 COALESCE((SELECT id FROM plans WHERE session_id = ?1), ?2),
                 ?1, ?3, ?4, ?5,
                 COALESCE((SELECT created_at FROM plans WHERE session_id = ?1), ?6),
                 ?6
             )",
            params![session_id, plan_id, plan.title, status, content, now],
        )?;

        // Get the actual plan ID (either new or existing)
        let actual_id: String = self.db.conn().query_row(
            "SELECT id FROM plans WHERE session_id = ?1",
            [session_id],
            |row| row.get(0),
        )?;

        tracing::info!(
            session_id = %session_id,
            plan_id = %actual_id,
            "Upserted plan"
        );

        Ok(actual_id)
    }

    /// Get plan for a session
    pub fn get_plan_for_session(&self, session_id: &str) -> Result<Option<PlanFile>> {
        let result = self.db.conn().query_row(
            "SELECT title, status, content, created_at
             FROM plans WHERE session_id = ?1",
            [session_id],
            |row| {
                Ok(PlanRow {
                    title: row.get(0)?,
                    status: row.get(1)?,
                    content: row.get(2)?,
                    created_at: row.get(3)?,
                })
            },
        );

        match result {
            Ok(row) => {
                let mut plan = PlanFile::from_markdown(&row.content)
                    .map_err(|e| anyhow::anyhow!("Failed to parse plan: {}", e))?;

                // Override with DB values (in case markdown parsing has different values)
                plan.title = row.title;
                plan.session_id = Some(session_id.to_string());

                if let Ok(status) = row.status.parse::<PlanStatus>() {
                    plan.status = status;
                }

                if let Ok(dt) = DateTime::parse_from_rfc3339(&row.created_at) {
                    plan.created_at = dt.with_timezone(&Utc);
                }

                Ok(Some(plan))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Delete plan by ID
    pub fn delete_plan(&self, plan_id: &str) -> Result<()> {
        self.db
            .conn()
            .execute("DELETE FROM plans WHERE id = ?1", [plan_id])?;
        tracing::info!(plan_id = %plan_id, "Deleted plan");
        Ok(())
    }

    /// Abandon plan for a session (deletes it, allowing new plan)
    pub fn abandon_plan(&self, session_id: &str) -> Result<bool> {
        let rows = self
            .db
            .conn()
            .execute("DELETE FROM plans WHERE session_id = ?1", [session_id])?;

        if rows > 0 {
            tracing::info!(session_id = %session_id, "Abandoned plan for session");
        }

        Ok(rows > 0)
    }

    /// Check if session has a plan
    pub fn has_plan(&self, session_id: &str) -> bool {
        self.db
            .conn()
            .query_row(
                "SELECT 1 FROM plans WHERE session_id = ?1",
                [session_id],
                |_| Ok(()),
            )
            .is_ok()
    }

    /// Update plan status
    pub fn update_status(&self, session_id: &str, status: PlanStatus) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db.conn().execute(
            "UPDATE plans SET status = ?1, updated_at = ?2 WHERE session_id = ?3",
            params![status.to_string(), now, session_id],
        )?;
        Ok(())
    }

    /// Update plan content (full markdown)
    pub fn update_content(&self, session_id: &str, plan: &PlanFile) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let content = plan.to_markdown();
        let status = plan.status.to_string();

        self.db.conn().execute(
            "UPDATE plans SET title = ?1, status = ?2, content = ?3, updated_at = ?4
             WHERE session_id = ?5",
            params![plan.title, status, content, now, session_id],
        )?;
        Ok(())
    }

    /// List all plans (for migration/debugging)
    pub fn list_all(&self) -> Result<Vec<PlanSummary>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT p.id, p.session_id, p.title, p.status, p.created_at, s.working_dir
             FROM plans p
             LEFT JOIN sessions s ON p.session_id = s.id
             ORDER BY p.updated_at DESC",
        )?;

        let plans = stmt.query_map([], plan_summary_from_row)?;

        plans.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// List completed plans scoped to a working directory.
    pub fn list_completed_for_working_dir(&self, working_dir: &str) -> Result<Vec<PlanSummary>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT p.id, p.session_id, p.title, p.status, p.created_at, s.working_dir
             FROM plans p
             INNER JOIN sessions s ON p.session_id = s.id
             WHERE p.status = ?1 AND s.working_dir = ?2
             ORDER BY p.updated_at DESC",
        )?;

        let plans = stmt.query_map(
            params![PlanStatus::Completed.to_string(), working_dir],
            plan_summary_from_row,
        )?;

        plans.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

fn plan_summary_from_row(row: &Row<'_>) -> rusqlite::Result<PlanSummary> {
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

/// Internal row type for plan queries
struct PlanRow {
    title: String,
    status: String,
    content: String,
    created_at: String,
}

/// Summary of a plan for listing
#[derive(Debug, Clone)]
pub struct PlanSummary {
    pub id: String,
    pub session_id: String,
    pub title: String,
    pub status: PlanStatus,
    pub created_at: String,
    pub working_dir: Option<String>,
}
