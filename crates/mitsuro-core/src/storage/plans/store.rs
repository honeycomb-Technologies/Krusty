use anyhow::Result;
use chrono::Utc;
use rusqlite::params;

use crate::plan::{PlanFile, PlanStatus};
use crate::storage::database::Database;

use super::model::{plan_summary_from_row, PlanRow, PlanSummary};

pub struct PlanStore<'a> {
    db: &'a Database,
}

impl<'a> PlanStore<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn upsert_plan(&self, session_id: &str, plan: &PlanFile) -> Result<String> {
        let now = Utc::now().to_rfc3339();
        let plan_id = uuid::Uuid::new_v4().to_string();
        let content = plan.to_markdown();
        let status = plan.status.to_string();

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
            Ok(row) => Ok(Some(row.into_plan_file(session_id)?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn delete_plan(&self, plan_id: &str) -> Result<()> {
        self.db
            .conn()
            .execute("DELETE FROM plans WHERE id = ?1", [plan_id])?;
        tracing::info!(plan_id = %plan_id, "Deleted plan");
        Ok(())
    }

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

    pub fn update_status(&self, session_id: &str, status: PlanStatus) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db.conn().execute(
            "UPDATE plans SET status = ?1, updated_at = ?2 WHERE session_id = ?3",
            params![status.to_string(), now, session_id],
        )?;
        Ok(())
    }

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
