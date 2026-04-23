use std::path::Path;

use anyhow::{Context, Result};
use tracing::{debug, warn};

pub(super) struct PendingTask {
    pub(super) task_id: String,
    pub(super) description: String,
}

const CREATE_TABLE_SQL: &str = "\
    CREATE TABLE IF NOT EXISTS autonomous_tasks (\
        id TEXT PRIMARY KEY,\
        session_id TEXT NOT NULL,\
        description TEXT NOT NULL,\
        status TEXT NOT NULL DEFAULT 'pending',\
        assigned_to TEXT,\
        result TEXT,\
        error TEXT,\
        created_at INTEGER NOT NULL DEFAULT (unixepoch()),\
        updated_at INTEGER NOT NULL DEFAULT (unixepoch())\
    )";

pub(super) async fn poll_next_task(
    db_path: &Path,
    session_id: &str,
    teammate_name: &str,
) -> Result<Option<PendingTask>> {
    let db_path = db_path.to_path_buf();
    let session_id = session_id.to_string();
    let teammate_name = teammate_name.to_string();

    tokio::task::spawn_blocking(move || {
        let db = crate::storage::Database::new(&db_path).context("open task store db")?;
        db.conn()
            .execute(CREATE_TABLE_SQL, [])
            .context("ensure autonomous_tasks table")?;

        let mut stmt = db
            .conn()
            .prepare(
                "UPDATE autonomous_tasks
                 SET status = 'running', assigned_to = ?1, updated_at = unixepoch()
                 WHERE id = (
                     SELECT id FROM autonomous_tasks
                     WHERE session_id = ?2 AND status = 'pending'
                     ORDER BY created_at ASC
                     LIMIT 1
                 )
                 RETURNING id, description",
            )
            .context("prepare claim query")?;

        let mut rows = stmt
            .query(rusqlite::params![teammate_name, session_id])
            .context("execute claim query")?;

        match rows.next().context("read claimed row")? {
            Some(row) => {
                let task_id: String = row.get(0).context("task id column")?;
                let description: String = row.get(1).context("description column")?;
                debug!(
                    teammate = %teammate_name,
                    task_id = %task_id,
                    "claimed task from store"
                );
                Ok(Some(PendingTask {
                    task_id,
                    description,
                }))
            }
            None => Ok(None),
        }
    })
    .await
    .context("spawn_blocking panicked")?
}

pub(super) async fn record_task_complete(db_path: &Path, task_id: &str, result: &str) {
    if let Err(error) = record_task_complete_inner(db_path, task_id, result).await {
        warn!(task_id = %task_id, error = %error, "failed to mark task complete");
    }
}

pub(super) async fn record_task_failed(db_path: &Path, task_id: &str, error: &str) {
    if let Err(update_error) = record_task_failed_inner(db_path, task_id, error).await {
        warn!(task_id = %task_id, error = %update_error, "failed to mark task failed");
    }
}

async fn record_task_complete_inner(db_path: &Path, task_id: &str, result: &str) -> Result<()> {
    let db_path = db_path.to_path_buf();
    let task_id = task_id.to_string();
    let result = result.to_string();

    tokio::task::spawn_blocking(move || {
        let db = crate::storage::Database::new(&db_path).context("open task store db")?;
        db.conn()
            .execute(
                "UPDATE autonomous_tasks SET status = 'completed', result = ?1, updated_at = unixepoch() WHERE id = ?2",
                rusqlite::params![result, task_id],
            )
            .context("mark task completed")?;
        Ok(())
    })
    .await
    .context("spawn_blocking panicked")?
}

async fn record_task_failed_inner(db_path: &Path, task_id: &str, error: &str) -> Result<()> {
    let db_path = db_path.to_path_buf();
    let task_id = task_id.to_string();
    let error = error.to_string();

    tokio::task::spawn_blocking(move || {
        let db = crate::storage::Database::new(&db_path).context("open task store db")?;
        db.conn()
            .execute(
                "UPDATE autonomous_tasks SET status = 'failed', error = ?1, updated_at = unixepoch() WHERE id = ?2",
                rusqlite::params![error, task_id],
            )
            .context("mark task failed")?;
        Ok(())
    })
    .await
    .context("spawn_blocking panicked")?
}
