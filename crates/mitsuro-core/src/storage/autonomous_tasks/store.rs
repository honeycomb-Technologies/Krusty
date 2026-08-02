use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

use crate::storage::database::Database;

use super::model::{AutonomousTask, TaskStatus};

pub struct AutonomousTaskStore {
    db: Database,
}

impl AutonomousTaskStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub fn create_task(
        &self,
        session_id: &str,
        subject: &str,
        description: &str,
        blocked_by: &[String],
    ) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let blocked_by_json =
            serde_json::to_string(blocked_by).context("serializing blocked_by")?;

        self.db.conn().execute(
            "INSERT INTO autonomous_tasks (id, session_id, subject, description, blocked_by)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, session_id, subject, description, blocked_by_json],
        )?;

        Ok(id)
    }

    pub fn claim_task(&self, task_id: &str, owner: &str) -> Result<()> {
        self.db.conn().execute(
            "UPDATE autonomous_tasks
             SET status = 'in_progress', owner = ?2, updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            params![task_id, owner],
        )?;
        Ok(())
    }

    pub fn complete_task(&self, task_id: &str, result: &str) -> Result<()> {
        self.db.conn().execute(
            "UPDATE autonomous_tasks
             SET status = 'completed', result = ?2,
                 completed_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            params![task_id, result],
        )?;
        Ok(())
    }

    pub fn fail_task(&self, task_id: &str, error: &str) -> Result<()> {
        self.db.conn().execute(
            "UPDATE autonomous_tasks
             SET status = 'failed', result = ?2,
                 completed_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            params![task_id, error],
        )?;
        Ok(())
    }

    pub fn list_tasks(&self, session_id: &str) -> Result<Vec<AutonomousTask>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, session_id, subject, description, status, owner,
                    blocked_by, created_at, updated_at, completed_at, result
             FROM autonomous_tasks
             WHERE session_id = ?1
             ORDER BY created_at ASC",
        )?;

        let rows = stmt.query_map(params![session_id], row_to_task)?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("reading autonomous tasks")
    }

    /// Return pending tasks whose blockers have all completed.
    pub fn get_available_tasks(&self, session_id: &str) -> Result<Vec<AutonomousTask>> {
        let all = self.list_tasks(session_id)?;

        let completed_ids: std::collections::HashSet<String> = all
            .iter()
            .filter(|t| t.status == TaskStatus::Completed)
            .map(|t| t.id.clone())
            .collect();

        Ok(all
            .into_iter()
            .filter(|t| {
                t.status == TaskStatus::Pending
                    && t.blocked_by.iter().all(|dep| completed_ids.contains(dep))
            })
            .collect())
    }

    pub fn get_task(&self, task_id: &str) -> Result<Option<AutonomousTask>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, session_id, subject, description, status, owner,
                    blocked_by, created_at, updated_at, completed_at, result
             FROM autonomous_tasks
             WHERE id = ?1",
        )?;

        stmt.query_row(params![task_id], row_to_task)
            .optional()
            .context("fetching autonomous task")
    }
}

fn row_to_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<AutonomousTask> {
    let status_str: String = row.get(4)?;
    let status = TaskStatus::parse(&status_str).unwrap_or(TaskStatus::Pending);

    let blocked_by_json: String = row.get(6)?;
    let blocked_by: Vec<String> = serde_json::from_str(&blocked_by_json).unwrap_or_default();

    Ok(AutonomousTask {
        id: row.get(0)?,
        session_id: row.get(1)?,
        subject: row.get(2)?,
        description: row.get(3)?,
        status,
        owner: row.get(5)?,
        blocked_by,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
        completed_at: row.get(9)?,
        result: row.get(10)?,
    })
}
