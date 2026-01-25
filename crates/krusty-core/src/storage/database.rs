//! SQLite database wrapper with versioned migrations

use anyhow::Result;
use rusqlite::Connection;
use std::path::Path;
use tracing::info;

/// Current schema version
const SCHEMA_VERSION: i32 = 11;

/// SQLite database wrapper
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Create a new database at the given path
    pub fn new(path: &Path) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(path)?;

        // Enable WAL mode for better concurrent access
        // This prevents lock contention when multiple instances try to access the database
        conn.pragma_update(None, "journal_mode", "WAL")?;

        // Set busy timeout to avoid immediate failures on lock contention
        conn.busy_timeout(std::time::Duration::from_secs(5))?;

        let db = Self { conn };
        db.run_migrations()?;
        Ok(db)
    }

    /// Get the underlying connection
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Get the current schema version from database
    pub(crate) fn get_schema_version(&self) -> i32 {
        // Create version table if it doesn't exist
        if let Err(e) = self.conn.execute(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        ) {
            tracing::warn!("Failed to create schema_version table: {}", e);
            // Table creation failed, assume version 0
            return 0;
        }

        self.conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    /// Set schema version after successful migration
    fn set_schema_version(&self, version: i32) -> Result<()> {
        self.conn.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            [version],
        )?;
        Ok(())
    }

    /// Run database migrations incrementally
    pub(crate) fn run_migrations(&self) -> Result<()> {
        let current_version = self.get_schema_version();
        info!(
            "Database schema version: {} (target: {})",
            current_version, SCHEMA_VERSION
        );

        if current_version >= SCHEMA_VERSION {
            return Ok(());
        }

        // Migration 1: Initial schema
        if current_version < 1 {
            info!("Running migration 1: Initial schema");
            self.conn.execute_batch(
                r#"
                -- Sessions table
                CREATE TABLE IF NOT EXISTS sessions (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    model TEXT,
                    working_dir TEXT
                );

                -- Messages table
                CREATE TABLE IF NOT EXISTS messages (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    session_id TEXT NOT NULL,
                    role TEXT NOT NULL,
                    content TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    tool_calls TEXT,
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
                );

                -- Index for faster message lookups
                CREATE INDEX IF NOT EXISTS idx_messages_session
                    ON messages(session_id);

                -- Index for session sorting
                CREATE INDEX IF NOT EXISTS idx_sessions_updated
                    ON sessions(updated_at DESC);

                -- User preferences
                CREATE TABLE IF NOT EXISTS user_preferences (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL,
                    updated_at INTEGER NOT NULL
                );
                "#,
            )?;
            self.set_schema_version(1)?;
        }

        // Migration 2: Add token_count to sessions
        if current_version < 2 {
            info!("Running migration 2: Add token_count to sessions");
            self.conn
                .execute_batch("ALTER TABLE sessions ADD COLUMN token_count INTEGER DEFAULT 0;")?;
            self.set_schema_version(2)?;
        }

        // Migration 3: Block UI state table for session restoration
        if current_version < 3 {
            info!("Running migration 3: Add block_ui_state table");
            self.conn.execute_batch(
                r#"
                -- Block UI state for session restoration
                -- Stores collapsed/expanded state and scroll position per block
                CREATE TABLE IF NOT EXISTS block_ui_state (
                    session_id TEXT NOT NULL,
                    block_id TEXT NOT NULL,
                    block_type TEXT NOT NULL,
                    collapsed INTEGER NOT NULL DEFAULT 1,
                    scroll_offset INTEGER NOT NULL DEFAULT 0,
                    PRIMARY KEY (session_id, block_id),
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
                );

                -- Index for fast lookup by session
                CREATE INDEX IF NOT EXISTS idx_block_ui_state_session
                    ON block_ui_state(session_id);
                "#,
            )?;
            self.set_schema_version(3)?;
        }

        // Migration 4: Pinch support
        if current_version < 4 {
            info!("Running migration 4: Pinch support");
            self.conn.execute_batch(
                r#"
                -- Add parent_session_id to sessions for chain tracking
                ALTER TABLE sessions ADD COLUMN parent_session_id TEXT REFERENCES sessions(id);

                -- File activity tracking for importance scoring
                CREATE TABLE IF NOT EXISTS file_activity (
                    session_id TEXT NOT NULL,
                    file_path TEXT NOT NULL,
                    read_count INTEGER NOT NULL DEFAULT 0,
                    write_count INTEGER NOT NULL DEFAULT 0,
                    edit_count INTEGER NOT NULL DEFAULT 0,
                    last_accessed TEXT NOT NULL,
                    user_referenced INTEGER NOT NULL DEFAULT 0,
                    PRIMARY KEY (session_id, file_path),
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
                );

                -- Index for fast file activity lookups
                CREATE INDEX IF NOT EXISTS idx_file_activity_session
                    ON file_activity(session_id);

                -- Pinch metadata for tracking context transfers
                CREATE TABLE IF NOT EXISTS pinch_metadata (
                    id TEXT PRIMARY KEY,
                    source_session_id TEXT NOT NULL,
                    target_session_id TEXT NOT NULL,
                    summary TEXT NOT NULL,
                    key_files TEXT NOT NULL,
                    user_preservation_hints TEXT,
                    user_direction TEXT,
                    created_at TEXT NOT NULL,
                    FOREIGN KEY (source_session_id) REFERENCES sessions(id),
                    FOREIGN KEY (target_session_id) REFERENCES sessions(id)
                );
                "#,
            )?;
            self.set_schema_version(4)?;
        }

        // Migration 5: Rename handoff_metadata to pinch_metadata
        if current_version < 5 {
            info!("Running migration 5: Rename to pinch_metadata");
            // Check if old table exists and rename it, or create new one
            let has_old_table: bool = self.conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='handoff_metadata'",
                [],
                |row| row.get(0),
            ).unwrap_or(0) > 0;

            if has_old_table {
                self.conn
                    .execute_batch("ALTER TABLE handoff_metadata RENAME TO pinch_metadata;")?;
            } else {
                // Create fresh if neither exists
                self.conn.execute_batch(
                    r#"
                    CREATE TABLE IF NOT EXISTS pinch_metadata (
                        id TEXT PRIMARY KEY,
                        source_session_id TEXT NOT NULL,
                        target_session_id TEXT NOT NULL,
                        summary TEXT NOT NULL,
                        key_files TEXT NOT NULL,
                        user_preservation_hints TEXT,
                        user_direction TEXT,
                        created_at TEXT NOT NULL,
                        FOREIGN KEY (source_session_id) REFERENCES sessions(id),
                        FOREIGN KEY (target_session_id) REFERENCES sessions(id)
                    );
                    "#,
                )?;
            }
            self.set_schema_version(5)?;
        }

        // Migration 6: Plans table for strict session-plan linkage
        if current_version < 6 {
            info!("Running migration 6: Plans table with session linkage");
            self.conn.execute_batch(
                r#"
                -- Plans table with strict 1:1 session linkage
                -- session_id UNIQUE enforces one plan per session
                -- ON DELETE CASCADE removes plan when session is deleted
                CREATE TABLE IF NOT EXISTS plans (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL UNIQUE,
                    title TEXT NOT NULL,
                    status TEXT NOT NULL DEFAULT 'in_progress',
                    content TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
                );

                -- Index for fast plan lookup by session
                CREATE INDEX IF NOT EXISTS idx_plans_session
                    ON plans(session_id);

                -- Index for listing plans by status
                CREATE INDEX IF NOT EXISTS idx_plans_status
                    ON plans(status);
                "#,
            )?;
            self.set_schema_version(6)?;
        }

        // Migration 7: User hooks table
        if current_version < 7 {
            info!("Running migration 7: User hooks table");
            self.conn.execute_batch(
                r#"
                -- User-configurable hooks for tool execution
                -- hook_type: PreToolUse, PostToolUse, Notification, UserPromptSubmit
                -- tool_pattern: regex pattern to match tool names (e.g., "Write|Edit", "Bash", ".*")
                -- command: shell command to execute (receives JSON on stdin)
                CREATE TABLE IF NOT EXISTS user_hooks (
                    id TEXT PRIMARY KEY,
                    hook_type TEXT NOT NULL,
                    tool_pattern TEXT NOT NULL,
                    command TEXT NOT NULL,
                    enabled INTEGER NOT NULL DEFAULT 1,
                    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                );

                -- Index for fast lookup by hook type
                CREATE INDEX IF NOT EXISTS idx_user_hooks_type
                    ON user_hooks(hook_type);
                "#,
            )?;
            self.set_schema_version(7)?;
        }

        // Migration 8: Agent state tracking for background execution
        if current_version < 8 {
            info!("Running migration 8: Agent state tracking");
            self.conn.execute_batch(
                r#"
                -- Add agent execution state to sessions
                -- agent_state: 'idle', 'streaming', 'tool_executing', 'awaiting_input', 'error'
                ALTER TABLE sessions ADD COLUMN agent_state TEXT NOT NULL DEFAULT 'idle';

                -- When the agent started processing (for monitoring)
                ALTER TABLE sessions ADD COLUMN agent_started_at TEXT;

                -- Last event time (for stale detection)
                ALTER TABLE sessions ADD COLUMN agent_last_event_at TEXT;

                -- Index for finding active sessions quickly
                CREATE INDEX IF NOT EXISTS idx_sessions_agent_state
                    ON sessions(agent_state) WHERE agent_state != 'idle';
                "#,
            )?;
            self.set_schema_version(8)?;
        }

        // Migration 9: Multi-tenant core tables (users, workspaces)
        if current_version < 9 {
            info!("Running migration 9: Multi-tenant core tables");
            self.conn.execute_batch(
                r#"
                -- Users table for multi-tenant SaaS
                CREATE TABLE IF NOT EXISTS users (
                    id TEXT PRIMARY KEY,
                    email TEXT NOT NULL UNIQUE,
                    display_name TEXT,
                    avatar_url TEXT,
                    tailscale_user_id TEXT UNIQUE,
                    oauth_subject TEXT UNIQUE,
                    license_tier TEXT NOT NULL DEFAULT 'free',
                    license_expires_at TEXT,
                    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    last_login_at TEXT
                );

                CREATE INDEX IF NOT EXISTS idx_users_email ON users(email);
                CREATE INDEX IF NOT EXISTS idx_users_tailscale ON users(tailscale_user_id);

                -- Workspaces (team containers)
                CREATE TABLE IF NOT EXISTS workspaces (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    slug TEXT NOT NULL UNIQUE,
                    owner_id TEXT NOT NULL REFERENCES users(id),
                    settings TEXT DEFAULT '{}',
                    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                );

                CREATE INDEX IF NOT EXISTS idx_workspaces_owner ON workspaces(owner_id);
                CREATE INDEX IF NOT EXISTS idx_workspaces_slug ON workspaces(slug);

                -- Workspace membership
                CREATE TABLE IF NOT EXISTS workspace_members (
                    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
                    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                    role TEXT NOT NULL DEFAULT 'member',
                    joined_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    PRIMARY KEY (workspace_id, user_id)
                );

                CREATE INDEX IF NOT EXISTS idx_workspace_members_user ON workspace_members(user_id);

                -- Usage tracking for billing
                CREATE TABLE IF NOT EXISTS usage_tracking (
                    id TEXT PRIMARY KEY,
                    workspace_id TEXT REFERENCES workspaces(id) ON DELETE CASCADE,
                    user_id TEXT NOT NULL REFERENCES users(id),
                    resource_type TEXT NOT NULL,
                    resource_id TEXT,
                    quantity INTEGER NOT NULL DEFAULT 1,
                    metadata TEXT,
                    period_start TEXT NOT NULL,
                    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                );

                CREATE INDEX IF NOT EXISTS idx_usage_workspace_period ON usage_tracking(workspace_id, period_start);
                CREATE INDEX IF NOT EXISTS idx_usage_user_period ON usage_tracking(user_id, period_start);
                "#,
            )?;
            self.set_schema_version(9)?;
        }

        // Migration 10: Add user_id columns to existing tables
        if current_version < 10 {
            info!("Running migration 10: Add user_id to existing tables");
            self.conn.execute_batch(
                r#"
                -- Add user_id to sessions
                ALTER TABLE sessions ADD COLUMN user_id TEXT REFERENCES users(id);
                ALTER TABLE sessions ADD COLUMN workspace_id TEXT REFERENCES workspaces(id);

                -- Add user_id to user_preferences (nullable for backwards compat)
                ALTER TABLE user_preferences ADD COLUMN user_id TEXT REFERENCES users(id);

                -- Add user_id to user_hooks
                ALTER TABLE user_hooks ADD COLUMN user_id TEXT REFERENCES users(id);
                ALTER TABLE user_hooks ADD COLUMN workspace_id TEXT REFERENCES workspaces(id);
                "#,
            )?;
            self.set_schema_version(10)?;
        }

        // Migration 11: Indexes for user-scoped queries
        if current_version < 11 {
            info!("Running migration 11: User-scoped indexes");
            self.conn.execute_batch(
                r#"
                CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);
                CREATE INDEX IF NOT EXISTS idx_sessions_workspace ON sessions(workspace_id);
                CREATE INDEX IF NOT EXISTS idx_sessions_user_workspace ON sessions(user_id, workspace_id);
                CREATE INDEX IF NOT EXISTS idx_prefs_user ON user_preferences(user_id);
                CREATE INDEX IF NOT EXISTS idx_hooks_user ON user_hooks(user_id);
                CREATE INDEX IF NOT EXISTS idx_hooks_workspace ON user_hooks(workspace_id);
                "#,
            )?;
            self.set_schema_version(11)?;
        }

        info!("Migrations complete");
        Ok(())
    }
}
