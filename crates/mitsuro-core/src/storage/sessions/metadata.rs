use anyhow::Result;
use chrono::Utc;
use rusqlite::params;

use crate::ai::models::ModelKey;
use crate::tools::registry::PermissionMode;

use super::{SessionManager, WorkMode, WorkspaceMode};

impl SessionManager {
    pub fn update_session_title(&self, session_id: &str, title: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        self.db.conn().execute(
            "UPDATE sessions SET title = ?1, updated_at = ?2 WHERE id = ?3",
            params![title, now, session_id],
        )?;

        Ok(())
    }

    /// Update session working directory.
    ///
    /// Pass `None` to clear the working directory.
    pub fn update_session_working_dir(
        &self,
        session_id: &str,
        working_dir: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        self.db.conn().execute(
            "UPDATE sessions SET working_dir = ?1, updated_at = ?2 WHERE id = ?3",
            params![working_dir, now, session_id],
        )?;

        Ok(())
    }

    /// Update the session workspace mode and active project directory.
    ///
    /// For explicit project modes we also align `working_dir` to the chosen
    /// project root so subsequent file tools execute within the active project.
    pub fn update_session_workspace(
        &self,
        session_id: &str,
        project_dir: Option<&str>,
        workspace_mode: WorkspaceMode,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let working_dir = match workspace_mode {
            WorkspaceMode::Neutral => None,
            WorkspaceMode::Selected | WorkspaceMode::Created => project_dir,
        };

        self.db.conn().execute(
            "UPDATE sessions
             SET project_dir = ?1,
                 workspace_mode = ?2,
                 working_dir = ?3,
                 updated_at = ?4
             WHERE id = ?5",
            params![
                project_dir,
                workspace_mode.to_string(),
                working_dir,
                now,
                session_id
            ],
        )?;

        Ok(())
    }

    /// Update the full persisted workspace contract.
    ///
    /// This is intentionally narrower than `update_session_workspace`: callers
    /// use it when both runtime `working_dir` and semantic `project_dir` are
    /// already normalized and must remain distinct.
    pub fn update_session_workspace_contract(
        &self,
        session_id: &str,
        working_dir: Option<&str>,
        project_dir: Option<&str>,
        workspace_mode: WorkspaceMode,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        self.db.conn().execute(
            "UPDATE sessions
             SET working_dir = ?1,
                 project_dir = ?2,
                 workspace_mode = ?3,
                 updated_at = ?4
             WHERE id = ?5",
            params![
                working_dir,
                project_dir,
                workspace_mode.to_string(),
                now,
                session_id
            ],
        )?;

        Ok(())
    }

    /// Update session work mode
    pub fn update_session_work_mode(&self, session_id: &str, work_mode: WorkMode) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        self.db.conn().execute(
            "UPDATE sessions SET work_mode = ?1, updated_at = ?2 WHERE id = ?3",
            params![work_mode.to_string(), now, session_id],
        )?;

        Ok(())
    }

    /// Update session model
    pub fn update_session_model(&self, session_id: &str, model: Option<&str>) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        self.db.conn().execute(
            "UPDATE sessions
             SET model = ?1,
                 model_key_json = NULL,
                 model_catalog_revision = NULL,
                 updated_at = ?2
             WHERE id = ?3",
            params![model, now, session_id],
        )?;

        Ok(())
    }

    /// Persist an exact model identity while retaining the legacy slug column.
    pub fn update_session_model_selection(
        &self,
        session_id: &str,
        key: Option<&ModelKey>,
        catalog_revision: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let model = key.map(|key| key.model_id.as_str());
        let key_json = key.map(serde_json::to_string).transpose()?;
        self.db.conn().execute(
            "UPDATE sessions
             SET model = ?1,
                 model_key_json = ?2,
                 model_catalog_revision = ?3,
                 updated_at = ?4
             WHERE id = ?5",
            params![model, key_json, catalog_revision, now, session_id],
        )?;
        Ok(())
    }

    /// Update optional target branch metadata for a session.
    pub fn update_session_target_branch(
        &self,
        session_id: &str,
        target_branch: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        self.db.conn().execute(
            "UPDATE sessions SET target_branch = ?1, updated_at = ?2 WHERE id = ?3",
            params![target_branch, now, session_id],
        )?;

        Ok(())
    }

    /// Update session permission mode.
    pub fn update_session_permission_mode(
        &self,
        session_id: &str,
        permission_mode: PermissionMode,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        self.db.conn().execute(
            "UPDATE sessions SET permission_mode = ?1, updated_at = ?2 WHERE id = ?3",
            params![permission_mode.as_str(), now, session_id],
        )?;

        Ok(())
    }

    /// Promote or demote a session in active conversation lists.
    pub fn update_session_pinned(&self, session_id: &str, pinned: bool) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db.conn().execute(
            "UPDATE sessions
             SET pinned_at = CASE WHEN ?1 THEN ?2 ELSE NULL END
             WHERE id = ?3",
            params![pinned, now, session_id],
        )?;
        Ok(())
    }

    /// Move a session into or out of the archive without deleting its history.
    pub fn update_session_archived(&self, session_id: &str, archived: bool) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db.conn().execute(
            "UPDATE sessions
             SET archived_at = CASE WHEN ?1 THEN ?2 ELSE NULL END
             WHERE id = ?3",
            params![archived, now, session_id],
        )?;
        Ok(())
    }

    /// Update session token count
    pub fn update_token_count(&self, session_id: &str, token_count: usize) -> Result<()> {
        self.db.conn().execute(
            "UPDATE sessions SET token_count = ?1 WHERE id = ?2",
            params![token_count as i64, session_id],
        )?;
        Ok(())
    }
}
