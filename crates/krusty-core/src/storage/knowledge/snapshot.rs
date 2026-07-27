use anyhow::Result;
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

use crate::storage::{
    AgentMemory, Database, MemoryNamespace, MemoryStore, MemoryType, ReportStore,
};

use super::activity::load_snapshot_activity;
use super::render::build_current_snapshot_content;
use super::CURRENT_SNAPSHOT_TITLE;

/// Materialized orientation derived from memories, reports, and recent work.
///
/// A knowledge snapshot is intentionally not an `AgentMemory`: it is generated
/// state that may be refreshed or removed at any time and must never compete
/// with canonical durable facts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnowledgeSnapshot {
    pub id: String,
    pub title: String,
    pub content: String,
    pub project_dir: Option<String>,
    pub user_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Detect legacy snapshots that were stored in `agent_memories` before
/// `knowledge_snapshots` became the canonical location.
pub fn is_current_snapshot(memory: &AgentMemory) -> bool {
    memory.memory_type == MemoryType::Project && is_current_snapshot_title(&memory.title)
}

/// Retained so legacy memory mutation surfaces can continue protecting the old
/// generated title during migration.
pub fn is_current_snapshot_title(title: &str) -> bool {
    title == CURRENT_SNAPSHOT_TITLE
}

pub fn get_current_snapshot(
    db_path: &Path,
    project_dir: Option<&str>,
    user_id: Option<&str>,
) -> Result<Option<KnowledgeSnapshot>> {
    let db = Database::new(db_path)?;
    load_snapshot(db.conn(), project_dir, user_id)
}

pub fn refresh_current_snapshot(
    db_path: &Path,
    project_dir: Option<&str>,
    user_id: Option<&str>,
) -> Result<Option<KnowledgeSnapshot>> {
    let memory_store = MemoryStore::new(Database::new(db_path)?);
    let report_store = ReportStore::new(Database::new(db_path)?);
    // Crew memories are intentionally excluded from this owner/project-wide
    // materialization. A crew member's private carry-forward context is
    // selected at request time, where its namespace id is available.
    let memories = memory_store
        .list_for_exact_owner(project_dir, user_id)
        .into_iter()
        .filter(|memory| memory.namespace != MemoryNamespace::Crew)
        .collect::<Vec<_>>();
    let reports = report_store.list_reports_for_exact_owner(project_dir, user_id)?;
    let (recent_runs, task_outcomes) = load_snapshot_activity(db_path, project_dir, user_id)?;

    let content = build_current_snapshot_content(
        &memories,
        &reports,
        &recent_runs,
        &task_outcomes,
        project_dir,
    );

    let db = Database::new(db_path)?;
    let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate)?;
    let existing = load_snapshot(&tx, project_dir, user_id)?;

    let Some(content) = content else {
        tx.execute(
            "DELETE FROM knowledge_snapshots
             WHERE project_dir IS ?1 AND user_id IS ?2",
            params![project_dir, user_id],
        )?;
        tx.commit()?;
        return Ok(None);
    };

    if let Some(existing) = existing.as_ref() {
        if existing.content == content {
            tx.commit()?;
            return Ok(Some(existing.clone()));
        }
        tx.execute(
            "UPDATE knowledge_snapshots
             SET title = ?1, content = ?2, updated_at = datetime('now')
             WHERE id = ?3",
            params![CURRENT_SNAPSHOT_TITLE, content, existing.id],
        )?;
    } else {
        tx.execute(
            "INSERT INTO knowledge_snapshots
                (id, title, content, project_dir, user_id)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                Uuid::new_v4().to_string(),
                CURRENT_SNAPSHOT_TITLE,
                content,
                project_dir,
                user_id,
            ],
        )?;
    }

    let snapshot = load_snapshot(&tx, project_dir, user_id)?
        .ok_or_else(|| anyhow::anyhow!("knowledge snapshot not found after refresh"))?;
    tx.commit()?;
    Ok(Some(snapshot))
}

fn load_snapshot(
    conn: &rusqlite::Connection,
    project_dir: Option<&str>,
    user_id: Option<&str>,
) -> Result<Option<KnowledgeSnapshot>> {
    Ok(conn
        .query_row(
            "SELECT id, title, content, project_dir, user_id, created_at, updated_at
             FROM knowledge_snapshots
             WHERE project_dir IS ?1 AND user_id IS ?2
             LIMIT 1",
            params![project_dir, user_id],
            |row| {
                Ok(KnowledgeSnapshot {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    content: row.get(2)?,
                    project_dir: row.get(3)?,
                    user_id: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            },
        )
        .optional()?)
}
