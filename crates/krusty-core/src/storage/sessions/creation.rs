use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, OptionalExtension};

use crate::agent::PinchContext;

use super::{SessionManager, SessionType, WorkspaceMode};

struct LinkedSessionContract {
    user_id: Option<String>,
    session_type: SessionType,
    working_dir: Option<String>,
    project_dir: Option<String>,
    workspace_mode: WorkspaceMode,
    target_branch: Option<String>,
}

impl SessionManager {
    pub fn create_session(
        &self,
        title: &str,
        model: Option<&str>,
        working_dir: Option<&str>,
    ) -> Result<String> {
        self.create_session_with_target_branch(title, model, working_dir, None)
    }

    /// Create a new session with optional target branch metadata.
    pub fn create_session_with_target_branch(
        &self,
        title: &str,
        model: Option<&str>,
        working_dir: Option<&str>,
        target_branch: Option<&str>,
    ) -> Result<String> {
        self.create_session_for_user_with_target_branch(
            title,
            model,
            working_dir,
            None,
            target_branch,
        )
    }

    /// Create a new session with user ownership (multi-tenant)
    pub fn create_session_for_user(
        &self,
        title: &str,
        model: Option<&str>,
        working_dir: Option<&str>,
        user_id: Option<&str>,
    ) -> Result<String> {
        self.create_session_for_user_with_target_branch(title, model, working_dir, user_id, None)
    }

    /// Create a new session with explicit workspace and surface metadata.
    pub fn create_session_for_user_with_config(
        &self,
        title: &str,
        model: Option<&str>,
        working_dir: Option<&str>,
        project_dir: Option<&str>,
        workspace_mode: WorkspaceMode,
        user_id: Option<&str>,
        target_branch: Option<&str>,
        session_type: SessionType,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        self.db.conn().execute(
            "INSERT INTO sessions (id, title, created_at, updated_at, model, working_dir, project_dir, workspace_mode, session_type, user_id, target_branch)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                id,
                title,
                now,
                now,
                model,
                working_dir,
                project_dir,
                workspace_mode.to_string(),
                session_type.to_string(),
                user_id,
                target_branch
            ],
        )?;

        Ok(id)
    }

    /// Create a new session with user ownership and optional target branch.
    pub fn create_session_for_user_with_target_branch(
        &self,
        title: &str,
        model: Option<&str>,
        working_dir: Option<&str>,
        user_id: Option<&str>,
        target_branch: Option<&str>,
    ) -> Result<String> {
        let project_dir = working_dir;
        let workspace_mode = if working_dir.is_some() {
            WorkspaceMode::Selected
        } else {
            WorkspaceMode::Neutral
        };
        self.create_session_for_user_with_config(
            title,
            model,
            working_dir,
            project_dir,
            workspace_mode,
            user_id,
            target_branch,
            SessionType::Code,
        )
    }

    /// Create a linked child session that inherits the parent's ownership metadata.
    pub fn create_linked_session(
        &self,
        title: &str,
        parent_session_id: &str,
        pinch_ctx: &PinchContext,
        model: Option<&str>,
        working_dir: Option<&str>,
        target_branch: Option<&str>,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let parent_contract = self
            .db
            .conn()
            .query_row(
                "SELECT user_id, session_type, working_dir, project_dir, workspace_mode, target_branch
                 FROM sessions WHERE id = ?1",
                [parent_session_id],
                |row| {
                    let session_type_raw: String = row.get(1)?;
                    let workspace_mode_raw: String = row.get(4)?;
                    Ok(LinkedSessionContract {
                        user_id: row.get(0)?,
                        session_type: session_type_raw.parse().unwrap_or(SessionType::Code),
                        working_dir: row.get(2)?,
                        project_dir: row.get(3)?,
                        workspace_mode: workspace_mode_raw.parse().unwrap_or_else(|_| {
                            let project_dir: Option<String> = row.get(3).ok().flatten();
                            let working_dir: Option<String> = row.get(2).ok().flatten();
                            if project_dir.is_some() || working_dir.is_some() {
                                WorkspaceMode::Selected
                            } else {
                                WorkspaceMode::Neutral
                            }
                        }),
                        target_branch: row.get(5)?,
                    })
                },
            )
            .optional()?;

        let fallback_workspace_mode = if working_dir.is_some() {
            WorkspaceMode::Selected
        } else {
            WorkspaceMode::Neutral
        };
        let contract = parent_contract.unwrap_or_else(|| LinkedSessionContract {
            user_id: None,
            session_type: SessionType::Code,
            working_dir: working_dir.map(ToOwned::to_owned),
            project_dir: working_dir.map(ToOwned::to_owned),
            workspace_mode: fallback_workspace_mode,
            target_branch: target_branch.map(ToOwned::to_owned),
        });

        // Create new session with parent reference
        self.db.conn().execute(
            "INSERT INTO sessions (id, title, created_at, updated_at, model, working_dir, project_dir, workspace_mode, session_type, user_id, parent_session_id, target_branch)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                id,
                title,
                now,
                now,
                model,
                contract.working_dir,
                contract.project_dir,
                contract.workspace_mode.to_string(),
                contract.session_type.to_string(),
                contract.user_id,
                parent_session_id,
                contract.target_branch
            ],
        )?;

        // Store pinch metadata
        let pinch_id = uuid::Uuid::new_v4().to_string();
        let key_files_json = serde_json::to_string(&pinch_ctx.ranked_files)?;

        self.db.conn().execute(
            "INSERT INTO pinch_metadata (id, source_session_id, target_session_id, summary, key_files, user_preservation_hints, user_direction, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                pinch_id,
                parent_session_id,
                id,
                &pinch_ctx.work_summary,
                key_files_json,
                &pinch_ctx.preservation_hints,
                &pinch_ctx.direction,
                now
            ],
        )?;

        Ok(id)
    }
}
