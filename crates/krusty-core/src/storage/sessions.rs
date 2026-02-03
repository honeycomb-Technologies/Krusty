//! Session CRUD operations

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::database::Database;
use crate::agent::PinchContext;

/// Session metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
    pub updated_at: DateTime<Utc>,
    pub token_count: Option<usize>,
    /// Parent session ID for linked sessions (pinch)
    pub parent_session_id: Option<String>,
    /// Working directory for this session
    pub working_dir: Option<String>,
    /// User ID for multi-tenant isolation
    pub user_id: Option<String>,
}

/// Session manager for CRUD operations
pub struct SessionManager {
    db: Database,
}

impl SessionManager {
    /// Create a new session manager
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Get reference to underlying database
    pub fn db(&self) -> &Database {
        &self.db
    }

    /// Create a new session
    pub fn create_session(
        &self,
        title: &str,
        model: Option<&str>,
        working_dir: Option<&str>,
    ) -> Result<String> {
        self.create_session_for_user(title, model, working_dir, None)
    }

    /// Create a new session with user ownership (multi-tenant)
    pub fn create_session_for_user(
        &self,
        title: &str,
        model: Option<&str>,
        working_dir: Option<&str>,
        user_id: Option<&str>,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        self.db.conn().execute(
            "INSERT INTO sessions (id, title, created_at, updated_at, model, working_dir, user_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, title, now, now, model, working_dir, user_id],
        )?;

        Ok(id)
    }

    /// List sessions, optionally filtered by working directory
    ///
    /// If `working_dir` is Some, only returns sessions from that directory.
    /// If None, returns all sessions.
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
        // Build WHERE clause and params based on provided filters
        let (where_clause, params): (String, Vec<String>) = match (working_dir, user_id) {
            (Some(dir), Some(uid)) => (
                "WHERE working_dir = ?1 AND user_id = ?2".to_string(),
                vec![dir.to_string(), uid.to_string()],
            ),
            (Some(dir), None) => ("WHERE working_dir = ?1".to_string(), vec![dir.to_string()]),
            (None, Some(uid)) => ("WHERE user_id = ?1".to_string(), vec![uid.to_string()]),
            (None, None) => (String::new(), vec![]),
        };

        let sql = format!(
            "SELECT id, title, updated_at, token_count, parent_session_id, working_dir, user_id
             FROM sessions {}
             ORDER BY updated_at DESC",
            where_clause
        );

        let params_refs: Vec<&dyn rusqlite::ToSql> =
            params.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        let mut stmt = self.db.conn().prepare(&sql)?;
        let sessions = stmt
            .query_map(params_refs.as_slice(), Self::map_session_row)?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(sessions)
    }

    /// Helper to map a row to SessionInfo
    fn map_session_row(row: &rusqlite::Row) -> rusqlite::Result<SessionInfo> {
        let updated_at: String = row.get(2)?;
        let token_count: Option<i64> = row.get(3)?;

        Ok(SessionInfo {
            id: row.get(0)?,
            title: row.get(1)?,
            updated_at: DateTime::parse_from_rfc3339(&updated_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            token_count: token_count.map(|t| t as usize),
            parent_session_id: row.get(4)?,
            working_dir: row.get(5)?,
            user_id: row.get(6)?,
        })
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
            let mut stmt = self.db.conn().prepare(
                "SELECT DISTINCT working_dir FROM sessions
                 WHERE working_dir IS NOT NULL AND user_id = ?1
                 ORDER BY working_dir",
            )?;
            let dirs = stmt.query_map([uid], |row| row.get(0))?;
            dirs.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        } else {
            let mut stmt = self.db.conn().prepare(
                "SELECT DISTINCT working_dir FROM sessions
                 WHERE working_dir IS NOT NULL
                 ORDER BY working_dir",
            )?;
            let dirs = stmt.query_map([], |row| row.get(0))?;
            dirs.collect::<Result<Vec<_>, _>>().map_err(Into::into)
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

        let mut stmt = self.db.conn().prepare(
            "SELECT id, title, updated_at, token_count, parent_session_id, working_dir, user_id
             FROM sessions
             WHERE working_dir IS NOT NULL
             ORDER BY working_dir, updated_at DESC",
        )?;

        let mut result: HashMap<String, Vec<SessionInfo>> = HashMap::new();

        let rows = stmt.query_map([], |row| {
            let updated_at: String = row.get(2)?;
            let token_count: Option<i64> = row.get(3)?;
            let working_dir: String = row.get(5)?;

            Ok((
                working_dir.clone(),
                SessionInfo {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    updated_at: DateTime::parse_from_rfc3339(&updated_at)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    token_count: token_count.map(|t| t as usize),
                    parent_session_id: row.get(4)?,
                    working_dir: Some(working_dir),
                    user_id: row.get(6)?,
                },
            ))
        })?;

        for row in rows {
            let (dir, session) = row?;
            result.entry(dir).or_default().push(session);
        }

        Ok(result)
    }

    /// Get a specific session
    pub fn get_session(&self, session_id: &str) -> Result<Option<SessionInfo>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, title, updated_at, token_count, parent_session_id, working_dir, user_id FROM sessions WHERE id = ?1",
        )?;

        let session = stmt.query_row([session_id], |row| {
            let updated_at: String = row.get(2)?;
            let token_count: Option<i64> = row.get(3)?;

            Ok(SessionInfo {
                id: row.get(0)?,
                title: row.get(1)?,
                updated_at: DateTime::parse_from_rfc3339(&updated_at)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                token_count: token_count.map(|t| t as usize),
                parent_session_id: row.get(4)?,
                working_dir: row.get(5)?,
                user_id: row.get(6)?,
            })
        });

        match session {
            Ok(s) => Ok(Some(s)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Update session title
    pub fn update_session_title(&self, session_id: &str, title: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        self.db.conn().execute(
            "UPDATE sessions SET title = ?1, updated_at = ?2 WHERE id = ?3",
            params![title, now, session_id],
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

    /// Delete a session and all its messages
    pub fn delete_session(&self, session_id: &str) -> Result<()> {
        // First, clear parent_session_id references from children (orphan them)
        // This prevents foreign key constraint violations
        self.db.conn().execute(
            "UPDATE sessions SET parent_session_id = NULL WHERE parent_session_id = ?1",
            params![session_id],
        )?;

        // Clear pinch_metadata references
        self.db.conn().execute(
            "DELETE FROM pinch_metadata WHERE source_session_id = ?1 OR target_session_id = ?1",
            params![session_id],
        )?;

        // Clear file_activity for this session
        self.db.conn().execute(
            "DELETE FROM file_activity WHERE session_id = ?1",
            params![session_id],
        )?;

        // Clear block_ui_state for this session
        self.db.conn().execute(
            "DELETE FROM block_ui_state WHERE session_id = ?1",
            params![session_id],
        )?;

        // Messages will be deleted via ON DELETE CASCADE
        self.db
            .conn()
            .execute("DELETE FROM sessions WHERE id = ?1", params![session_id])?;

        tracing::info!(session_id = %session_id, "Session deleted from database");
        Ok(())
    }

    /// Save a message to a session
    /// The content field stores JSON-serialized Vec<Content> for full fidelity
    pub fn save_message(&self, session_id: &str, role: &str, content_json: &str) -> Result<()> {
        super::messages::MessageStore::new(&self.db).save_message(session_id, role, content_json)
    }

    /// Load all messages for a session
    /// Returns (role, content_json) pairs where content_json can be deserialized to Vec<Content>
    pub fn load_session_messages(&self, session_id: &str) -> Result<Vec<(String, String)>> {
        super::messages::MessageStore::new(&self.db).load_session_messages(session_id)
    }

    /// Generate a title from the first message content
    /// Truncates at word boundaries for cleaner display
    /// Uses char-based indexing for UTF-8 safety
    pub fn generate_title_from_content(content: &str) -> String {
        // Use first line only, cleaned up
        let first_line = content.lines().next().unwrap_or("").trim();

        // Count chars (not bytes) for UTF-8 safety
        let char_count = first_line.chars().count();

        // If short enough, use as-is
        if char_count <= 50 {
            return first_line.to_string();
        }

        // Get first 50 chars and find last word boundary
        let first_50: String = first_line.chars().take(50).collect();
        if let Some(last_space) = first_50.rfind(char::is_whitespace) {
            // last_space is a byte index in first_50, but first_50 is already truncated
            // So we can safely slice it
            let char_idx = first_50[..last_space].chars().count();
            if char_idx > 20 {
                // Only use word boundary if we keep at least 20 chars
                let prefix: String = first_line.chars().take(char_idx).collect();
                return format!("{}...", prefix.trim_end());
            }
        }

        // Fallback: hard truncate at 47 chars
        let truncated: String = first_line.chars().take(47).collect();
        format!("{}...", truncated)
    }

    // =========================================================================
    // Block UI State
    // =========================================================================

    /// Save block UI state (collapsed, scroll_offset) for a block
    pub fn save_block_ui_state(
        &self,
        session_id: &str,
        block_id: &str,
        collapsed: bool,
        scroll_offset: u16,
    ) -> Result<()> {
        super::block_ui::BlockUiStore::new(&self.db).save_block_ui_state(
            session_id,
            block_id,
            collapsed,
            scroll_offset,
        )
    }

    /// Load all block UI states for a session
    pub fn load_block_ui_states(&self, session_id: &str) -> Vec<super::block_ui::BlockUiState> {
        super::block_ui::BlockUiStore::new(&self.db).load_block_ui_states(session_id)
    }

    // =========================================================================
    // Linked Sessions (Pinch)
    // =========================================================================

    /// Create a new session linked to a parent (for pinch)
    ///
    /// The new session starts fresh but with a reference to its parent
    /// and pinch metadata preserved for context.
    pub fn create_linked_session(
        &self,
        title: &str,
        parent_session_id: &str,
        pinch_ctx: &PinchContext,
        model: Option<&str>,
        working_dir: Option<&str>,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        // Create new session with parent reference
        self.db.conn().execute(
            "INSERT INTO sessions (id, title, created_at, updated_at, model, working_dir, parent_session_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, title, now, now, model, working_dir, parent_session_id],
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

    // =========================================================================
    // Agent State Tracking (for background execution)
    // =========================================================================

    /// Set the agent execution state for a session
    ///
    /// Valid states: "idle", "streaming", "tool_executing", "awaiting_input", "error"
    pub fn set_agent_state(&self, session_id: &str, state: &str) -> Result<()> {
        super::agent_state::AgentStateStore::new(&self.db).set_agent_state(session_id, state)
    }

    /// Get the agent state for a session
    pub fn get_agent_state(&self, session_id: &str) -> Option<super::agent_state::AgentState> {
        super::agent_state::AgentStateStore::new(&self.db).get_agent_state(session_id)
    }

    /// Update agent last_event_at timestamp (for keeping session "alive")
    pub fn touch_agent_event(&self, session_id: &str) -> Result<()> {
        super::agent_state::AgentStateStore::new(&self.db).touch_agent_event(session_id)
    }

    /// List sessions with active agents (not idle)
    pub fn list_active_sessions(&self) -> Result<Vec<(String, super::agent_state::AgentState)>> {
        super::agent_state::AgentStateStore::new(&self.db).list_active_sessions()
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::params;
    use tempfile::TempDir;

    use crate::storage::sessions::SessionManager;
    use crate::storage::Database;

    /// Helper to create a temporary database for testing
    fn create_test_db() -> (Database, TempDir) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db_path = temp_dir.path().join("test.db");
        let db = Database::new(&db_path).expect("Failed to create database");
        (db, temp_dir)
    }

    /// Helper to create a test user (required for multi-tenant tests)
    fn create_test_user(db: &Database, user_id: &str) {
        db.conn()
            .execute(
                "INSERT INTO users (id, email, license_tier) VALUES (?1, ?2, ?3)",
                params![user_id, format!("{user_id}@example.com"), "free"],
            )
            .expect("Failed to create user");
    }

    #[test]
    fn test_session_ownership_single_tenant_mode() {
        // In single-tenant mode (user_id = None), any session is accessible
        let (db, _temp) = create_test_db();
        let manager = SessionManager::new(db);

        // Create session without user (single-tenant mode)
        let session_id = manager
            .create_session("Test Session", Some("claude-3-5-sonnet"), Some("/tmp"))
            .expect("Failed to create session");

        // Verify ownership - should succeed (no user check)
        let result = manager
            .verify_session_ownership(&session_id, None)
            .expect("Failed to verify ownership");

        assert!(
            result,
            "Single-tenant mode should allow access to any session"
        );
    }

    #[test]
    fn test_session_ownership_multi_tenant_mode_success() {
        // In multi-tenant mode, users can only access their own sessions
        let (db, _temp) = create_test_db();

        let user_id = "user-123";

        // Create the user first (required for foreign key constraint)
        create_test_user(&db, user_id);

        let manager = SessionManager::new(db);

        // Create session with user ownership
        let session_id = manager
            .create_session_for_user(
                "Test Session",
                Some("claude-3-5-sonnet"),
                Some("/tmp"),
                Some(user_id),
            )
            .expect("Failed to create session");

        // Verify ownership with correct user - should succeed
        let result = manager
            .verify_session_ownership(&session_id, Some(user_id))
            .expect("Failed to verify ownership");

        assert!(result, "User should have access to their own session");
    }

    #[test]
    fn test_session_ownership_multi_tenant_mode_cross_user_denied() {
        // Users cannot access sessions belonging to other users
        let (db, _temp) = create_test_db();

        let user_id = "user-123";
        let other_user_id = "user-456";

        // Create the users first
        create_test_user(&db, user_id);
        create_test_user(&db, other_user_id);

        let manager = SessionManager::new(db);

        // Create session with user ownership
        let session_id = manager
            .create_session_for_user(
                "Test Session",
                Some("claude-3-5-sonnet"),
                Some("/tmp"),
                Some(user_id),
            )
            .expect("Failed to create session");

        // Verify ownership with different user - should fail
        let result = manager
            .verify_session_ownership(&session_id, Some(other_user_id))
            .expect("Failed to verify ownership");

        assert!(
            !result,
            "User should NOT have access to another user's session"
        );
    }

    #[test]
    fn test_session_ownership_nonexistent_session() {
        // Non-existent sessions should fail ownership verification
        let (db, _temp) = create_test_db();
        let manager = SessionManager::new(db);

        let fake_session_id = uuid::Uuid::new_v4().to_string();

        // Single-tenant mode - nonexistent session
        let result_single = manager
            .verify_session_ownership(&fake_session_id, None)
            .expect("Failed to verify ownership");

        assert!(
            !result_single,
            "Non-existent session should not pass ownership check"
        );

        // Multi-tenant mode - nonexistent session
        let result_multi = manager
            .verify_session_ownership(&fake_session_id, Some("user-123"))
            .expect("Failed to verify ownership");

        assert!(
            !result_multi,
            "Non-existent session should not pass ownership check"
        );
    }

    #[test]
    fn test_session_ownership_mixed_users_isolation() {
        // Multiple users should have complete isolation
        let (db, _temp) = create_test_db();

        let user1 = "alice";
        let user2 = "bob";

        // Create the users first
        create_test_user(&db, user1);
        create_test_user(&db, user2);

        let manager = SessionManager::new(db);

        // Create sessions for different users
        let session1 = manager
            .create_session_for_user(
                "Alice's Session",
                Some("claude-3-5-sonnet"),
                Some("/tmp"),
                Some(user1),
            )
            .expect("Failed to create session for user1");

        let session2 = manager
            .create_session_for_user(
                "Bob's Session",
                Some("claude-3-5-sonnet"),
                Some("/tmp"),
                Some(user2),
            )
            .expect("Failed to create session for user2");

        // User 1 can only access their own sessions
        let user1_access_1 = manager
            .verify_session_ownership(&session1, Some(user1))
            .expect("Failed to verify ownership");
        let user1_access_2 = manager
            .verify_session_ownership(&session2, Some(user1))
            .expect("Failed to verify ownership");

        assert!(user1_access_1, "Alice should access her own session");
        assert!(!user1_access_2, "Alice should NOT access Bob's session");

        // User 2 can only access their own sessions
        let user2_access_1 = manager
            .verify_session_ownership(&session1, Some(user2))
            .expect("Failed to verify ownership");
        let user2_access_2 = manager
            .verify_session_ownership(&session2, Some(user2))
            .expect("Failed to verify ownership");

        assert!(!user2_access_1, "Bob should NOT access Alice's session");
        assert!(user2_access_2, "Bob should access his own session");
    }

    #[test]
    fn test_list_sessions_for_user_filters_by_user_id() {
        // list_sessions_for_user should only return sessions owned by the user
        let (db, _temp) = create_test_db();

        let user1 = "alice";
        let user2 = "bob";

        // Create the users first
        create_test_user(&db, user1);
        create_test_user(&db, user2);

        let manager = SessionManager::new(db);

        // Create sessions for different users
        let _session1 = manager
            .create_session_for_user(
                "Alice's Session 1",
                Some("claude-3-5-sonnet"),
                Some("/tmp"),
                Some(user1),
            )
            .expect("Failed to create session");
        let _session2 = manager
            .create_session_for_user(
                "Alice's Session 2",
                Some("claude-3-5-sonnet"),
                Some("/home"),
                Some(user1),
            )
            .expect("Failed to create session");
        let session3 = manager
            .create_session_for_user(
                "Bob's Session",
                Some("claude-3-5-sonnet"),
                Some("/tmp"),
                Some(user2),
            )
            .expect("Failed to create session");

        // User 1 should see only their 2 sessions
        let user1_sessions = manager
            .list_sessions_for_user(None, Some(user1))
            .expect("Failed to list sessions");

        assert_eq!(
            user1_sessions.len(),
            2,
            "User 1 should see exactly 2 sessions"
        );

        // User 2 should see only their 1 session
        let user2_sessions = manager
            .list_sessions_for_user(None, Some(user2))
            .expect("Failed to list sessions");

        assert_eq!(
            user2_sessions.len(),
            1,
            "User 2 should see exactly 1 session"
        );
        assert_eq!(
            user2_sessions[0].id, session3,
            "User 2 should see their own session"
        );
    }

    #[test]
    fn test_list_sessions_for_user_filters_by_working_dir_and_user() {
        // Combined filtering by working_dir AND user_id
        let (db, _temp) = create_test_db();

        let user = "alice";

        // Create the user first
        create_test_user(&db, user);

        let manager = SessionManager::new(db);

        // Create sessions in different directories
        let session1 = manager
            .create_session_for_user(
                "Session in /tmp",
                Some("claude-3-5-sonnet"),
                Some("/tmp"),
                Some(user),
            )
            .expect("Failed to create session");
        let _session2 = manager
            .create_session_for_user(
                "Session in /home",
                Some("claude-3-5-sonnet"),
                Some("/home"),
                Some(user),
            )
            .expect("Failed to create session");

        // Filter by both user and directory
        let tmp_sessions = manager
            .list_sessions_for_user(Some("/tmp"), Some(user))
            .expect("Failed to list sessions");

        assert_eq!(
            tmp_sessions.len(),
            1,
            "Should see exactly 1 session in /tmp"
        );
        assert_eq!(tmp_sessions[0].id, session1, "Should be the /tmp session");
    }

    #[test]
    fn test_get_session() {
        // Test retrieving a session by ID
        let (db, _temp) = create_test_db();
        let manager = SessionManager::new(db);

        let session_id = manager
            .create_session("Test Session", Some("claude-3-5-sonnet"), Some("/tmp"))
            .expect("Failed to create session");

        // Retrieve the session
        let session = manager
            .get_session(&session_id)
            .expect("Failed to get session");

        assert!(session.is_some(), "Session should exist");
        let session = session.unwrap();
        assert_eq!(session.id, session_id);
        assert_eq!(session.title, "Test Session");
        assert_eq!(session.working_dir, Some("/tmp".to_string()));
    }

    #[test]
    fn test_update_session_title() {
        // Test updating session title
        let (db, _temp) = create_test_db();
        let manager = SessionManager::new(db);

        let session_id = manager
            .create_session("Original Title", Some("claude-3-5-sonnet"), Some("/tmp"))
            .expect("Failed to create session");

        // Update the title
        manager
            .update_session_title(&session_id, "Updated Title")
            .expect("Failed to update title");

        // Verify the update
        let session = manager
            .get_session(&session_id)
            .expect("Failed to get session")
            .expect("Session should exist");

        assert_eq!(session.title, "Updated Title");
    }

    #[test]
    fn test_delete_session() {
        // Test deleting a session
        let (db, _temp) = create_test_db();
        let manager = SessionManager::new(db);

        let session_id = manager
            .create_session("Test Session", Some("claude-3-5-sonnet"), Some("/tmp"))
            .expect("Failed to create session");

        // Delete the session
        manager
            .delete_session(&session_id)
            .expect("Failed to delete session");

        // Session should be gone
        let session = manager
            .get_session(&session_id)
            .expect("Failed to get session");

        assert!(session.is_none(), "Session should be deleted");
    }

    #[test]
    fn test_agent_state_management() {
        // Test agent state tracking and updates
        let (db, _temp) = create_test_db();
        let manager = SessionManager::new(db);

        let session_id = manager
            .create_session("Test Session", Some("claude-3-5-sonnet"), Some("/tmp"))
            .expect("Failed to create session");

        // Initially idle
        let state = manager.get_agent_state(&session_id);
        assert!(state.is_some(), "Should have agent state");
        assert_eq!(state.unwrap().state, "idle", "Initial state should be idle");

        // Update to streaming
        manager
            .set_agent_state(&session_id, "streaming")
            .expect("Failed to update agent state");

        let state = manager.get_agent_state(&session_id).unwrap();
        assert_eq!(state.state, "streaming");
        assert!(state.started_at.is_some(), "Should have started_at");
        assert!(state.last_event_at.is_some(), "Should have last_event_at");

        // Update back to idle
        manager
            .set_agent_state(&session_id, "idle")
            .expect("Failed to update agent state");

        let state = manager.get_agent_state(&session_id).unwrap();
        assert_eq!(state.state, "idle");
        assert!(state.started_at.is_none(), "Idle should clear started_at");
    }

    #[test]
    fn test_list_active_sessions() {
        // Test filtering sessions by agent activity
        let (db, _temp) = create_test_db();
        let manager = SessionManager::new(db);

        let session1 = manager
            .create_session("Active 1", Some("claude-3-5-sonnet"), Some("/tmp"))
            .expect("Failed to create session");
        let session2 = manager
            .create_session("Active 2", Some("claude-3-5-sonnet"), Some("/tmp"))
            .expect("Failed to create session");
        let _session3 = manager
            .create_session("Idle", Some("claude-3-5-sonnet"), Some("/tmp"))
            .expect("Failed to create session");

        // Set two sessions to active states
        manager
            .set_agent_state(&session1, "streaming")
            .expect("Failed to update state");
        manager
            .set_agent_state(&session2, "tool_executing")
            .expect("Failed to update state");

        // List active sessions
        let active = manager
            .list_active_sessions()
            .expect("Failed to list active sessions");

        assert_eq!(active.len(), 2, "Should have 2 active sessions");

        let active_ids: Vec<&str> = active.iter().map(|(id, _)| id.as_str()).collect();
        assert!(active_ids.contains(&session1.as_str()));
        assert!(active_ids.contains(&session2.as_str()));
    }

    #[test]
    fn test_touch_agent_event() {
        // Test updating agent activity timestamp
        let (db, _temp) = create_test_db();
        let manager = SessionManager::new(db);

        let session_id = manager
            .create_session("Test Session", Some("claude-3-5-sonnet"), Some("/tmp"))
            .expect("Failed to create session");

        manager
            .set_agent_state(&session_id, "streaming")
            .expect("Failed to set streaming");

        // Touch the event
        manager
            .touch_agent_event(&session_id)
            .expect("Failed to touch event");

        let state = manager.get_agent_state(&session_id).unwrap();
        assert!(state.last_event_at.is_some(), "Should have last_event_at");
    }
}
