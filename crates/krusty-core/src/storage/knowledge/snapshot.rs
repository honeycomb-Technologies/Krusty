use anyhow::Result;
use std::path::Path;

use crate::storage::{AgentMemory, Database, MemoryStore, MemoryType, ReportStore};

use super::activity::load_snapshot_activity;
use super::render::build_current_snapshot_content;
use super::CURRENT_SNAPSHOT_TITLE;

pub fn is_current_snapshot(memory: &AgentMemory) -> bool {
    memory.memory_type == MemoryType::Project && is_current_snapshot_title(&memory.title)
}

pub fn is_current_snapshot_title(title: &str) -> bool {
    title == CURRENT_SNAPSHOT_TITLE
}

pub fn refresh_current_snapshot(
    db_path: &Path,
    project_dir: Option<&str>,
    user_id: Option<&str>,
) -> Result<Option<AgentMemory>> {
    let memory_store = MemoryStore::new(Database::new(db_path)?);
    let report_store = ReportStore::new(Database::new(db_path)?);
    let memories = memory_store.list(project_dir, user_id);
    let reports = report_store.list_reports_for_user(project_dir, user_id)?;
    let (recent_runs, task_outcomes) = load_snapshot_activity(db_path, project_dir, user_id)?;

    let Some(content) = build_current_snapshot_content(
        &memories,
        &reports,
        &recent_runs,
        &task_outcomes,
        project_dir,
    ) else {
        if let Some(existing) =
            memory_store.find_by_title_in_exact_scope(CURRENT_SNAPSHOT_TITLE, project_dir, user_id)
        {
            memory_store.delete(&existing.id)?;
        }
        return Ok(None);
    };

    let (snapshot, _) = memory_store.save_or_update_by_title(
        MemoryType::Project,
        CURRENT_SNAPSHOT_TITLE,
        &content,
        project_dir,
        user_id,
    )?;

    Ok(Some(snapshot))
}
