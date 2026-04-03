//! Lightweight task list for Mako agent coordination
//!
//! Tracks tasks within an autonomous session, including dependency edges
//! (blocked_by) so the orchestrator can schedule work in the right order.

use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::database::Database;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

impl TaskStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "in_progress" => Some(Self::InProgress),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousTask {
    pub id: String,
    pub session_id: String,
    pub subject: String,
    pub description: String,
    pub status: TaskStatus,
    pub owner: Option<String>,
    pub blocked_by: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
    pub result: Option<String>,
}

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

#[cfg(test)]
mod tests {
    use rusqlite::params;
    use tempfile::TempDir;

    use super::*;
    use crate::storage::Database;

    fn create_store() -> (AutonomousTaskStore, TempDir) {
        let tmp = TempDir::new().expect("temp dir");
        let db = Database::new(&tmp.path().join("tasks.db")).expect("db");
        let now = chrono::Utc::now().to_rfc3339();
        db.conn()
            .execute(
                "INSERT INTO sessions (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
                params!["sess-1", "Task Test", now, now],
            )
            .expect("seed session");
        (AutonomousTaskStore::new(db), tmp)
    }

    #[test]
    fn create_and_list_tasks() {
        let (store, _tmp) = create_store();
        let id = store
            .create_task("sess-1", "Write parser", "Implement the SQL parser", &[])
            .unwrap();
        assert!(!id.is_empty());

        let tasks = store.list_tasks("sess-1").unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].subject, "Write parser");
        assert_eq!(tasks[0].status, TaskStatus::Pending);
    }

    #[test]
    fn claim_complete_fail_lifecycle() {
        let (store, _tmp) = create_store();
        let t1 = store.create_task("sess-1", "Task A", "", &[]).unwrap();
        let t2 = store.create_task("sess-1", "Task B", "", &[]).unwrap();

        store.claim_task(&t1, "agent-1").unwrap();
        let task = store.get_task(&t1).unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::InProgress);
        assert_eq!(task.owner.as_deref(), Some("agent-1"));

        store.complete_task(&t1, "done").unwrap();
        let task = store.get_task(&t1).unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Completed);
        assert!(task.completed_at.is_some());

        store.fail_task(&t2, "compile error").unwrap();
        let task = store.get_task(&t2).unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Failed);
        assert_eq!(task.result.as_deref(), Some("compile error"));
    }

    #[test]
    fn get_available_respects_blocked_by() {
        let (store, _tmp) = create_store();
        let t1 = store.create_task("sess-1", "Foundation", "", &[]).unwrap();
        let t2 = store
            .create_task("sess-1", "Depends on foundation", "", &[t1.clone()])
            .unwrap();
        let _t3 = store.create_task("sess-1", "Independent", "", &[]).unwrap();

        // Before completing t1, only t1 and t3 are available
        let available = store.get_available_tasks("sess-1").unwrap();
        assert_eq!(available.len(), 2);
        assert!(available.iter().all(|t| t.id != t2));

        // After completing t1, t2 becomes available
        store.complete_task(&t1, "ok").unwrap();
        let available = store.get_available_tasks("sess-1").unwrap();
        assert_eq!(available.len(), 2);
        assert!(available.iter().any(|t| t.id == t2));
    }

    #[test]
    fn get_task_returns_none_for_missing() {
        let (store, _tmp) = create_store();
        assert!(store.get_task("nonexistent").unwrap().is_none());
    }
}
