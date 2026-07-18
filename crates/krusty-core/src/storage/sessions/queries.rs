use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::params;
use rusqlite::types::ValueRef;

use crate::tools::registry::PermissionMode;

use super::{SessionInfo, SessionManager, SessionType, WorkspaceMode};

const LIST_SESSIONS_SQL_ALL: &str =
    "SELECT id, title, updated_at, token_count, parent_session_id, working_dir, user_id, work_mode, model, target_branch, project_dir, workspace_mode, session_type, permission_mode
             FROM sessions
             ORDER BY updated_at DESC";
const LIST_SESSIONS_SQL_BY_DIR: &str =
    "SELECT id, title, updated_at, token_count, parent_session_id, working_dir, user_id, work_mode, model, target_branch, project_dir, workspace_mode, session_type, permission_mode
             FROM sessions
             WHERE working_dir = ?1
             ORDER BY updated_at DESC";
const LIST_SESSIONS_SQL_BY_USER: &str =
    "SELECT id, title, updated_at, token_count, parent_session_id, working_dir, user_id, work_mode, model, target_branch, project_dir, workspace_mode, session_type, permission_mode
             FROM sessions
             WHERE user_id = ?1
             ORDER BY updated_at DESC";
const LIST_SESSIONS_SQL_BY_DIR_AND_USER: &str =
    "SELECT id, title, updated_at, token_count, parent_session_id, working_dir, user_id, work_mode, model, target_branch, project_dir, workspace_mode, session_type, permission_mode
             FROM sessions
             WHERE working_dir = ?1 AND user_id = ?2
             ORDER BY updated_at DESC";
const LIST_SESSIONS_SQL_BY_TYPE: &str =
    "SELECT id, title, updated_at, token_count, parent_session_id, working_dir, user_id, work_mode, model, target_branch, project_dir, workspace_mode, session_type, permission_mode
             FROM sessions
             WHERE session_type = ?1
             ORDER BY updated_at DESC";
const LIST_SESSIONS_SQL_BY_DIR_AND_TYPE: &str =
    "SELECT id, title, updated_at, token_count, parent_session_id, working_dir, user_id, work_mode, model, target_branch, project_dir, workspace_mode, session_type, permission_mode
             FROM sessions
             WHERE working_dir = ?1 AND session_type = ?2
             ORDER BY updated_at DESC";
const LIST_SESSIONS_SQL_BY_USER_AND_TYPE: &str =
    "SELECT id, title, updated_at, token_count, parent_session_id, working_dir, user_id, work_mode, model, target_branch, project_dir, workspace_mode, session_type, permission_mode
             FROM sessions
             WHERE user_id = ?1 AND session_type = ?2
             ORDER BY updated_at DESC";
const LIST_SESSIONS_SQL_BY_DIR_USER_AND_TYPE: &str =
    "SELECT id, title, updated_at, token_count, parent_session_id, working_dir, user_id, work_mode, model, target_branch, project_dir, workspace_mode, session_type, permission_mode
             FROM sessions
             WHERE working_dir = ?1 AND user_id = ?2 AND session_type = ?3
             ORDER BY updated_at DESC";
const LIST_SESSION_DIRS_SQL_ALL: &str = "SELECT DISTINCT working_dir FROM sessions
                 WHERE working_dir IS NOT NULL
                 ORDER BY working_dir";
const LIST_SESSION_DIRS_SQL_BY_USER: &str = "SELECT DISTINCT working_dir FROM sessions
                 WHERE working_dir IS NOT NULL AND user_id = ?1
                 ORDER BY working_dir";
const LIST_SESSIONS_BY_DIRECTORY_SQL: &str =
    "SELECT id, title, updated_at, token_count, parent_session_id, working_dir, user_id, work_mode, model, target_branch, project_dir, workspace_mode, session_type, permission_mode
             FROM sessions
             WHERE working_dir IS NOT NULL
             ORDER BY working_dir, updated_at DESC";
const GET_SESSION_SQL: &str =
    "SELECT id, title, updated_at, token_count, parent_session_id, working_dir, user_id, work_mode, model, target_branch, project_dir, workspace_mode, session_type, permission_mode
             FROM sessions
             WHERE id = ?1";

impl SessionManager {
    pub fn list_sessions(&self, working_dir: Option<&str>) -> Result<Vec<SessionInfo>> {
        self.list_sessions_for_user(working_dir, None)
    }

    /// List sessions for a specific user (multi-tenant)
    ///
    /// If `user_id` is Some, only returns sessions owned by that user.
    /// If `working_dir` is Some, also filters by that directory.
    pub fn list_sessions_for_user(
        &self,
        working_dir: Option<&str>,
        user_id: Option<&str>,
    ) -> Result<Vec<SessionInfo>> {
        match (working_dir, user_id) {
            (Some(dir), Some(uid)) => {
                self.collect_sessions(LIST_SESSIONS_SQL_BY_DIR_AND_USER, [dir, uid])
            }
            (Some(dir), None) => self.collect_sessions(LIST_SESSIONS_SQL_BY_DIR, [dir]),
            (None, Some(uid)) => self.collect_sessions(LIST_SESSIONS_SQL_BY_USER, [uid]),
            (None, None) => self.collect_sessions(LIST_SESSIONS_SQL_ALL, []),
        }
    }

    /// List sessions for a specific user and surface type, with optional directory filtering.
    pub fn list_sessions_for_user_by_type(
        &self,
        working_dir: Option<&str>,
        user_id: Option<&str>,
        session_type: SessionType,
    ) -> Result<Vec<SessionInfo>> {
        let session_type = session_type.to_string();
        match (working_dir, user_id) {
            (Some(dir), Some(uid)) => self.collect_sessions(
                LIST_SESSIONS_SQL_BY_DIR_USER_AND_TYPE,
                [dir, uid, &session_type],
            ),
            (Some(dir), None) => {
                self.collect_sessions(LIST_SESSIONS_SQL_BY_DIR_AND_TYPE, [dir, &session_type])
            }
            (None, Some(uid)) => {
                self.collect_sessions(LIST_SESSIONS_SQL_BY_USER_AND_TYPE, [uid, &session_type])
            }
            (None, None) => self.collect_sessions(LIST_SESSIONS_SQL_BY_TYPE, [&session_type]),
        }
    }

    fn collect_sessions<P>(&self, sql: &str, params: P) -> Result<Vec<SessionInfo>>
    where
        P: rusqlite::Params,
    {
        let mut stmt = self.db.conn().prepare(sql)?;
        let sessions = stmt
            .query_map(params, Self::map_session_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(sessions)
    }

    fn collect_string_column<P>(&self, sql: &str, params: P) -> Result<Vec<String>>
    where
        P: rusqlite::Params,
    {
        let mut stmt = self.db.conn().prepare(sql)?;
        let values = stmt.query_map(params, |row| row.get(0))?;
        values
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Helper to map a row to SessionInfo
    fn map_session_row(row: &rusqlite::Row) -> rusqlite::Result<SessionInfo> {
        let token_count: Option<i64> = row.get(3)?;
        let work_mode_raw: String = row.get(7)?;
        let model: Option<String> = row.get(8)?;
        let target_branch: Option<String> = row.get(9)?;
        let project_dir: Option<String> = row.get(10)?;
        let workspace_mode_raw: String = row.get(11)?;
        let session_type_raw: String = row.get(12)?;
        let permission_mode_raw: String = row.get(13).unwrap_or_else(|_| "autonomous".to_string());
        let working_dir: Option<String> = row.get(5)?;
        let workspace_mode = workspace_mode_raw.parse().unwrap_or_else(|_| {
            if project_dir.is_some() || working_dir.is_some() {
                WorkspaceMode::Selected
            } else {
                WorkspaceMode::Neutral
            }
        });
        let session_type = session_type_raw.parse().unwrap_or_default();

        Ok(SessionInfo {
            id: row.get(0)?,
            title: row.get(1)?,
            updated_at: parse_session_timestamp(row, 2)?,
            token_count: token_count.map(|t| t as usize),
            parent_session_id: row.get(4)?,
            working_dir,
            project_dir,
            workspace_mode,
            session_type,
            user_id: row.get(6)?,
            work_mode: work_mode_raw.parse().unwrap_or_default(),
            model,
            target_branch,
            permission_mode: match permission_mode_raw.as_str() {
                "supervised" => PermissionMode::Supervised,
                "autonomous" => PermissionMode::Autonomous,
                _ => PermissionMode::Autonomous,
            },
        })
    }

    fn map_session_row_with_directory(
        row: &rusqlite::Row,
    ) -> rusqlite::Result<(String, SessionInfo)> {
        let session = Self::map_session_row(row)?;
        let directory = session.working_dir.clone().ok_or_else(|| {
            rusqlite::Error::InvalidColumnType(
                5,
                "working_dir".to_string(),
                rusqlite::types::Type::Null,
            )
        })?;
        Ok((directory, session))
    }

    pub(super) fn map_active_session_row(
        row: &rusqlite::Row,
    ) -> rusqlite::Result<(SessionInfo, super::super::agent_state::AgentState)> {
        let session = Self::map_session_row(row)?;
        let agent_state = super::super::agent_state::AgentState {
            state: row.get(14)?,
            started_at: row.get(15)?,
            last_event_at: row.get(16)?,
        };
        Ok((session, agent_state))
    }

    /// List all directories that have sessions
    ///
    /// Returns sorted list of unique working directories.
    pub fn list_session_directories(&self) -> Result<Vec<String>> {
        self.list_session_directories_for_user(None)
    }

    /// List directories for a specific user (multi-tenant)
    pub fn list_session_directories_for_user(&self, user_id: Option<&str>) -> Result<Vec<String>> {
        if let Some(uid) = user_id {
            self.collect_string_column(LIST_SESSION_DIRS_SQL_BY_USER, [uid])
        } else {
            self.collect_string_column(LIST_SESSION_DIRS_SQL_ALL, [])
        }
    }

    /// Verify session belongs to user (multi-tenant ownership check)
    ///
    /// Returns true if the session exists and belongs to the specified user.
    /// Returns true for any session if user_id is None (single-tenant mode).
    pub fn verify_session_ownership(
        &self,
        session_id: &str,
        user_id: Option<&str>,
    ) -> Result<bool> {
        if let Some(uid) = user_id {
            let count: i64 = self.db.conn().query_row(
                "SELECT COUNT(*) FROM sessions WHERE id = ?1 AND user_id = ?2",
                params![session_id, uid],
                |row| row.get(0),
            )?;
            Ok(count > 0)
        } else {
            // Single-tenant mode - just check session exists
            let count: i64 = self.db.conn().query_row(
                "SELECT COUNT(*) FROM sessions WHERE id = ?1",
                params![session_id],
                |row| row.get(0),
            )?;
            Ok(count > 0)
        }
    }

    /// Get sessions grouped by directory
    ///
    /// Returns a map of directory -> sessions for tree display.
    pub fn list_sessions_by_directory(
        &self,
    ) -> Result<std::collections::HashMap<String, Vec<SessionInfo>>> {
        use std::collections::HashMap;

        let mut stmt = self.db.conn().prepare(LIST_SESSIONS_BY_DIRECTORY_SQL)?;

        let mut result: HashMap<String, Vec<SessionInfo>> = HashMap::new();

        let rows = stmt.query_map([], Self::map_session_row_with_directory)?;

        for row in rows {
            let (dir, session) = row?;
            result.entry(dir).or_default().push(session);
        }

        Ok(result)
    }

    /// Get a specific session
    pub fn get_session(&self, session_id: &str) -> Result<Option<SessionInfo>> {
        let mut stmt = self.db.conn().prepare(GET_SESSION_SQL)?;

        let session = stmt.query_row([session_id], Self::map_session_row);

        match session {
            Ok(s) => Ok(Some(s)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

fn parse_session_timestamp(row: &rusqlite::Row, index: usize) -> rusqlite::Result<DateTime<Utc>> {
    let value = row.get_ref(index)?;
    match value {
        ValueRef::Text(text) => {
            let text = std::str::from_utf8(text).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    index,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            DateTime::parse_from_rfc3339(text)
                .map(|date| date.with_timezone(&Utc))
                .or_else(|_| {
                    text.parse::<i64>()
                        .ok()
                        .and_then(|timestamp| DateTime::<Utc>::from_timestamp(timestamp, 0))
                        .ok_or(())
                })
                .map_err(|_| {
                    rusqlite::Error::InvalidColumnType(
                        index,
                        "updated_at".to_string(),
                        rusqlite::types::Type::Text,
                    )
                })
        }
        ValueRef::Integer(timestamp) => {
            DateTime::<Utc>::from_timestamp(timestamp, 0).ok_or_else(|| {
                rusqlite::Error::InvalidColumnType(
                    index,
                    "updated_at".to_string(),
                    rusqlite::types::Type::Integer,
                )
            })
        }
        _ => Err(rusqlite::Error::InvalidColumnType(
            index,
            "updated_at".to_string(),
            value.data_type(),
        )),
    }
}
