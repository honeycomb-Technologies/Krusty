use std::path::PathBuf;

use anyhow::{ensure, Context, Result};
use mitsuro_core::storage::{HiveRunExecutionContextV1, WorkerConversationLane, WorkspaceMode};
use rusqlite::Connection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkerConversationExecutionBinding {
    pub(crate) context: HiveRunExecutionContextV1,
    pub(crate) working_dir: Option<String>,
    pub(crate) project_dir: Option<String>,
}

/// Freeze one ordinary Worker run to the exact persisted conversation
/// workspace. Direct DMs inherit an explicit selected/created attachment while
/// hidden group lanes remain neutral. No daemon cwd fallback is representable.
pub(crate) fn resolve_worker_conversation_execution_binding(
    connection: &Connection,
    session_id: &str,
    worker_id: &str,
    worker_revision: u64,
    lane: WorkerConversationLane,
) -> Result<WorkerConversationExecutionBinding> {
    let (mode, working_dir, project_dir): (String, Option<String>, Option<String>) = connection
        .query_row(
            "SELECT workspace_mode, working_dir, project_dir
             FROM sessions WHERE id = ?1 AND session_type = 'hive'",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .with_context(|| format!("loading Worker conversation workspace {session_id}"))?;
    let mode = mode
        .parse::<WorkspaceMode>()
        .map_err(anyhow::Error::msg)
        .context("invalid Worker conversation workspace mode")?;

    if matches!(lane, WorkerConversationLane::Group { .. }) {
        ensure!(
            mode == WorkspaceMode::Neutral && working_dir.is_none() && project_dir.is_none(),
            "hidden Worker group lane carries a filesystem workspace"
        );
        return Ok(WorkerConversationExecutionBinding {
            context: HiveRunExecutionContextV1::worker_conversation_neutral(
                worker_id,
                worker_revision,
                lane,
            )?,
            working_dir: None,
            project_dir: None,
        });
    }

    match mode {
        WorkspaceMode::Neutral => {
            ensure!(
                working_dir.is_none() && project_dir.is_none(),
                "neutral Worker DM carries filesystem paths"
            );
            Ok(WorkerConversationExecutionBinding {
                context: HiveRunExecutionContextV1::worker_conversation_neutral(
                    worker_id,
                    worker_revision,
                    lane,
                )?,
                working_dir: None,
                project_dir: None,
            })
        }
        WorkspaceMode::Selected | WorkspaceMode::Created => {
            let working = exact_canonical_directory(working_dir.as_deref(), "working_dir")?;
            let project = exact_canonical_directory(project_dir.as_deref(), "project_dir")?;
            ensure!(
                working == project,
                "Worker Goal v1 requires one exact working/project directory"
            );
            Ok(WorkerConversationExecutionBinding {
                context: HiveRunExecutionContextV1::worker_workspace_attached(
                    worker_id,
                    worker_revision,
                    lane,
                    mode,
                    working.clone(),
                    Some(project.clone()),
                )?,
                working_dir: Some(working),
                project_dir: Some(project),
            })
        }
    }
}

fn exact_canonical_directory(value: Option<&str>, field: &str) -> Result<String> {
    let value = value.context(format!("attached Worker conversation has no {field}"))?;
    ensure!(
        !value.is_empty() && value.len() <= 16 * 1024 && !value.as_bytes().contains(&0),
        "attached Worker conversation {field} is invalid"
    );
    let path = PathBuf::from(value);
    ensure!(
        path.is_absolute(),
        "Worker conversation {field} is not absolute"
    );
    let canonical = path
        .canonicalize()
        .with_context(|| format!("resolving Worker conversation {field}"))?;
    ensure!(
        canonical.is_dir(),
        "Worker conversation {field} is not a directory"
    );
    let canonical = canonical.to_string_lossy().into_owned();
    ensure!(
        canonical == value,
        "Worker conversation {field} is not stored canonically"
    );
    Ok(canonical)
}
