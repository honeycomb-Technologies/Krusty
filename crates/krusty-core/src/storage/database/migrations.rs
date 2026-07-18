use anyhow::{ensure, Context, Result};
use rusqlite::{Transaction, TransactionBehavior};
use tracing::{debug, info};

use super::{Database, SCHEMA_VERSION};

impl Database {
    /// Get the current schema version from database
    #[cfg(test)]
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

    /// Set schema version within a transaction
    fn set_schema_version_tx(&self, tx: &rusqlite::Transaction, version: i32) -> Result<()> {
        tx.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            [version],
        )?;
        Ok(())
    }

    /// Check if a column exists in a table (for safe ALTER TABLE migrations).
    fn column_exists(tx: &rusqlite::Transaction, table: &str, column: &str) -> bool {
        tx.prepare(&format!("SELECT {} FROM {} LIMIT 0", column, table))
            .is_ok()
    }

    /// Check if a table exists (for data-cleanup migrations against lazily-created tables).
    fn table_exists(tx: &rusqlite::Transaction, table: &str) -> bool {
        tx.query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
            [table],
            |_| Ok(()),
        )
        .is_ok()
    }

    fn checkpoint_wal_without_busy_readers(&self, phase: &str) -> Result<()> {
        let (busy, log_frames, checkpointed_frames) = self
            .conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .with_context(|| format!("{phase} Mako privacy WAL checkpoint"))?;
        ensure!(
            busy == 0,
            "{phase} Mako privacy WAL checkpoint was busy (log_frames={log_frames}, checkpointed_frames={checkpointed_frames})"
        );
        Ok(())
    }

    /// Return a privacy-migration connection to normal WAL locking and force
    /// one database access so SQLite actually releases the exclusive lock.
    /// Merely changing `locking_mode` updates the requested mode; the lock is
    /// retained until the connection performs a subsequent database access.
    fn restore_normal_locking_after_privacy_migration(&self) -> Result<()> {
        self.conn
            .pragma_update(None, "locking_mode", "NORMAL")
            .context("restoring normal SQLite locking after Mako privacy migration")?;
        self.conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get::<_, i32>(0),
            )
            .context("releasing exclusive SQLite lock after Mako privacy migration")?;
        Ok(())
    }

    /// Run database migrations incrementally
    pub(crate) fn run_migrations(&self) -> Result<()> {
        // The steady-state path is read-only. Runtime stores open short-lived
        // connections frequently, so do not acquire the global SQLite writer
        // lock once this process has observed the target schema.
        let observed_version = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get::<_, i32>(0),
            )
            .ok();
        if observed_version.is_some_and(|version| version >= SCHEMA_VERSION) {
            return Ok(());
        }

        // Migration 43 rewrites secret-bearing legacy journal pages. Schema
        // 44 is committed only after the post-transaction checkpoint/VACUUM
        // completes, so a crash at any point before physical erasure resumes
        // cleanup on the next open instead of treating redaction as finished.
        let privacy_cleanup_requested =
            observed_version.is_some_and(|version| version > 0 && version < 44);
        if privacy_cleanup_requested {
            self.conn
                .pragma_update(None, "secure_delete", "ON")
                .context("enabling secure deletion for Mako privacy migration")?;
            self.conn
                .pragma_update(None, "locking_mode", "EXCLUSIVE")
                .context("reserving exclusive access for Mako privacy migration")?;
        }

        // The HTTP process and the independently supervised Mako daemon can
        // open the same database at the same time. Acquire the SQLite write
        // reservation before reading the version so a waiter observes every
        // migration committed by the process that won the startup race.
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)
            .context("acquiring database migration lock")?;
        tx.execute(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )
        .context("ensuring database schema version table")?;
        let current_version: i32 = tx
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get::<_, i32>(0),
            )
            .context("reading database schema version under migration lock")?;
        debug!(
            "Database schema version: {} (target: {})",
            current_version, SCHEMA_VERSION
        );

        // A missing version table and a transient read blocked by another
        // process look identical during the optimistic preflight. Once the
        // immediate transaction reveals an existing pre-44 database, release
        // the shared migration reservation and restart under the privacy
        // migration's exclusive/secure-delete policy. Fresh version-0
        // databases never request exclusive locking, so concurrent first opens
        // serialize on the ordinary writer lock instead of deadlocking.
        if current_version > 0 && current_version < 44 && !privacy_cleanup_requested {
            tx.commit()?;
            self.conn
                .pragma_update(None, "secure_delete", "ON")
                .context("enabling secure deletion for Mako privacy migration")?;
            self.conn
                .pragma_update(None, "locking_mode", "EXCLUSIVE")
                .context("reserving exclusive access for Mako privacy migration")?;
            return self.run_migrations();
        }

        if current_version >= SCHEMA_VERSION {
            tx.commit()?;
            if privacy_cleanup_requested {
                self.restore_normal_locking_after_privacy_migration()?;
            }
            return Ok(());
        }
        // The optimistic version read above may have failed while another
        // process held SQLite's migration lock. The value read under this
        // transaction is authoritative for deciding whether physical cleanup
        // is required; never publish 44 merely because preflight assumed 0.
        let privacy_cleanup_required = current_version > 0 && current_version < 44;

        // Migration 1: Initial schema
        if current_version < 1 {
            info!("Running migration 1: Initial schema");
            tx.execute_batch(
                r#"
                -- Sessions table
                CREATE TABLE IF NOT EXISTS sessions (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    model TEXT,
                    working_dir TEXT,
                    session_type TEXT NOT NULL DEFAULT 'code'
                        CHECK (session_type IN ('chat', 'code', 'mako')),
                    permission_mode TEXT NOT NULL DEFAULT 'autonomous'
                        CHECK (permission_mode IN ('supervised', 'autonomous'))
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
            self.set_schema_version_tx(&tx, 1)?;
        }

        // Migration 2: Add token_count to sessions
        if current_version < 2 {
            info!("Running migration 2: Add token_count to sessions");
            tx.execute_batch("ALTER TABLE sessions ADD COLUMN token_count INTEGER DEFAULT 0;")?;
            self.set_schema_version_tx(&tx, 2)?;
        }

        // Migration 3: Block UI state table for session restoration
        if current_version < 3 {
            info!("Running migration 3: Add block_ui_state table");
            tx.execute_batch(
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
            self.set_schema_version_tx(&tx, 3)?;
        }

        // Migration 4: Pinch support
        if current_version < 4 {
            info!("Running migration 4: Pinch support");
            tx.execute_batch(
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
            self.set_schema_version_tx(&tx, 4)?;
        }

        // Migration 5: Rename handoff_metadata to pinch_metadata
        if current_version < 5 {
            info!("Running migration 5: Rename to pinch_metadata");
            // Check if old table exists and rename it, or create new one
            let has_old_table: bool = tx.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='handoff_metadata'",
                [],
                |row| row.get(0),
            ).unwrap_or(0) > 0;

            if has_old_table {
                tx.execute_batch("ALTER TABLE handoff_metadata RENAME TO pinch_metadata;")?;
            } else {
                // Create fresh if neither exists
                tx.execute_batch(
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
            self.set_schema_version_tx(&tx, 5)?;
        }

        // Migration 6: Plans table for strict session-plan linkage
        if current_version < 6 {
            info!("Running migration 6: Plans table with session linkage");
            tx.execute_batch(
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
            self.set_schema_version_tx(&tx, 6)?;
        }

        // Migration 7: User hooks table
        if current_version < 7 {
            info!("Running migration 7: User hooks table");
            tx.execute_batch(
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
            self.set_schema_version_tx(&tx, 7)?;
        }

        // Migration 8: Agent state tracking for background execution
        if current_version < 8 {
            info!("Running migration 8: Agent state tracking");
            tx.execute_batch(
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
            self.set_schema_version_tx(&tx, 8)?;
        }

        // Migration 9: Multi-tenant core tables (users, workspaces)
        if current_version < 9 {
            info!("Running migration 9: Multi-tenant core tables");
            tx.execute_batch(
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
            self.set_schema_version_tx(&tx, 9)?;
        }

        // Migration 10: Add user_id columns to existing tables
        if current_version < 10 {
            info!("Running migration 10: Add user_id to existing tables");
            tx.execute_batch(
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
            self.set_schema_version_tx(&tx, 10)?;
        }

        // Migration 11: Indexes for user-scoped queries
        if current_version < 11 {
            info!("Running migration 11: User-scoped indexes");
            tx.execute_batch(
                r#"
                CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);
                CREATE INDEX IF NOT EXISTS idx_sessions_workspace ON sessions(workspace_id);
                CREATE INDEX IF NOT EXISTS idx_sessions_user_workspace ON sessions(user_id, workspace_id);
                CREATE INDEX IF NOT EXISTS idx_prefs_user ON user_preferences(user_id);
                CREATE INDEX IF NOT EXISTS idx_hooks_user ON user_hooks(user_id);
                CREATE INDEX IF NOT EXISTS idx_hooks_workspace ON user_hooks(workspace_id);
                "#,
            )?;
            self.set_schema_version_tx(&tx, 11)?;
        }

        // Migration 12: Smart Codebase Memory System
        if current_version < 12 {
            info!("Running migration 12: Smart Codebase Memory System");
            self.conn.execute_batch(
                r#"
                -- Codebases: First-class codebase entity
                CREATE TABLE IF NOT EXISTS codebases (
                    id TEXT PRIMARY KEY,
                    path TEXT NOT NULL UNIQUE,
                    name TEXT NOT NULL,
                    indexed_at TEXT,
                    index_version INTEGER NOT NULL DEFAULT 0,
                    config TEXT DEFAULT '{}'
                );

                CREATE INDEX IF NOT EXISTS idx_codebases_path ON codebases(path);

                -- Codebase index: Semantic code symbol index
                CREATE TABLE IF NOT EXISTS codebase_index (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    codebase_id TEXT NOT NULL REFERENCES codebases(id) ON DELETE CASCADE,
                    symbol_type TEXT NOT NULL,
                    symbol_name TEXT NOT NULL,
                    symbol_path TEXT NOT NULL,
                    file_path TEXT NOT NULL,
                    line_start INTEGER NOT NULL,
                    line_end INTEGER NOT NULL,
                    signature TEXT,
                    summary TEXT,
                    embedding BLOB,
                    calls TEXT DEFAULT '[]',
                    indexed_at TEXT NOT NULL
                );

                CREATE INDEX IF NOT EXISTS idx_codebase_index_codebase ON codebase_index(codebase_id);
                CREATE INDEX IF NOT EXISTS idx_codebase_index_symbol ON codebase_index(symbol_name);
                CREATE INDEX IF NOT EXISTS idx_codebase_index_file ON codebase_index(file_path);
                CREATE INDEX IF NOT EXISTS idx_codebase_index_type ON codebase_index(symbol_type);

                -- Codebase insights: Accumulated knowledge from sessions
                CREATE TABLE IF NOT EXISTS codebase_insights (
                    id TEXT PRIMARY KEY,
                    codebase_id TEXT NOT NULL REFERENCES codebases(id) ON DELETE CASCADE,
                    insight_type TEXT NOT NULL,
                    content TEXT NOT NULL,
                    embedding BLOB,
                    confidence REAL NOT NULL DEFAULT 0.5,
                    source_session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
                    access_count INTEGER NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL,
                    last_accessed_at TEXT NOT NULL
                );

                CREATE INDEX IF NOT EXISTS idx_codebase_insights_codebase ON codebase_insights(codebase_id);
                CREATE INDEX IF NOT EXISTS idx_codebase_insights_type ON codebase_insights(insight_type);
                CREATE INDEX IF NOT EXISTS idx_codebase_insights_confidence ON codebase_insights(confidence DESC);

                -- Session memories: Session-level learnings (may promote to insights)
                CREATE TABLE IF NOT EXISTS session_memories (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                    memory_type TEXT NOT NULL,
                    content TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    promoted_to_insight_id TEXT REFERENCES codebase_insights(id) ON DELETE SET NULL
                );

                CREATE INDEX IF NOT EXISTS idx_session_memories_session ON session_memories(session_id);
                CREATE INDEX IF NOT EXISTS idx_session_memories_type ON session_memories(memory_type);

                -- Link sessions to codebases
                ALTER TABLE sessions ADD COLUMN codebase_id TEXT REFERENCES codebases(id);
                "#,
            )?;
            self.set_schema_version_tx(&tx, 12)?;
        }

        // Migration 13: Push notification subscriptions
        if current_version < 13 {
            info!("Running migration 13: Push notification subscriptions");
            tx.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS push_subscriptions (
                    id TEXT PRIMARY KEY,
                    user_id TEXT,
                    endpoint TEXT NOT NULL,
                    p256dh TEXT NOT NULL,
                    auth TEXT NOT NULL,
                    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    last_used_at TEXT,
                    UNIQUE(endpoint)
                );

                CREATE INDEX IF NOT EXISTS idx_push_subscriptions_user
                    ON push_subscriptions(user_id);
                "#,
            )?;
            self.set_schema_version_tx(&tx, 13)?;
        }

        // Migration 14: Push delivery observability + subscription health fields
        if current_version < 14 {
            info!("Running migration 14: Push delivery observability");
            tx.execute_batch(
                r#"
                ALTER TABLE push_subscriptions ADD COLUMN last_success_at TEXT;
                ALTER TABLE push_subscriptions ADD COLUMN last_failure_at TEXT;
                ALTER TABLE push_subscriptions ADD COLUMN last_failure_reason TEXT;
                ALTER TABLE push_subscriptions ADD COLUMN failure_count INTEGER NOT NULL DEFAULT 0;

                CREATE TABLE IF NOT EXISTS push_delivery_attempts (
                    id TEXT PRIMARY KEY,
                    user_id TEXT,
                    session_id TEXT,
                    endpoint_hash TEXT NOT NULL,
                    provider_host TEXT NOT NULL,
                    event_type TEXT NOT NULL,
                    outcome TEXT NOT NULL,
                    http_status INTEGER,
                    error_message TEXT,
                    latency_ms INTEGER,
                    created_at TEXT NOT NULL
                );

                CREATE INDEX IF NOT EXISTS idx_push_delivery_attempts_user_created
                    ON push_delivery_attempts(user_id, created_at DESC);
                CREATE INDEX IF NOT EXISTS idx_push_delivery_attempts_outcome_created
                    ON push_delivery_attempts(outcome, created_at DESC);
                CREATE INDEX IF NOT EXISTS idx_push_delivery_attempts_session_created
                    ON push_delivery_attempts(session_id, created_at DESC);
                "#,
            )?;
            self.set_schema_version_tx(&tx, 14)?;
        }

        // Migration 15: Persist session work mode
        if current_version < 15 {
            info!("Running migration 15: Session work mode");
            tx.execute_batch(
                r#"
                ALTER TABLE sessions ADD COLUMN work_mode TEXT NOT NULL DEFAULT 'build'
                    CHECK (work_mode IN ('build', 'plan'));

                CREATE INDEX IF NOT EXISTS idx_sessions_work_mode
                    ON sessions(work_mode);
                "#,
            )?;
            self.set_schema_version_tx(&tx, 15)?;
        }

        // Migration 16: Optional target git branch on sessions
        if current_version < 16 {
            info!("Running migration 16: Session target branch");
            tx.execute_batch(
                r#"
                ALTER TABLE sessions ADD COLUMN target_branch TEXT;

                CREATE INDEX IF NOT EXISTS idx_sessions_target_branch
                    ON sessions(target_branch);
                "#,
            )?;
            self.set_schema_version_tx(&tx, 16)?;
        }

        // Migration 17: Context ledger + continuation contract persistence
        if current_version < 17 {
            info!("Running migration 17: Context continuation state");
            tx.execute_batch(
                r#"
                ALTER TABLE sessions ADD COLUMN context_ledger_json TEXT;
                ALTER TABLE sessions ADD COLUMN continuation_json TEXT;
                "#,
            )?;
            self.set_schema_version_tx(&tx, 17)?;
        }

        // Migration 18: Typed interrupted-turn recovery state
        if current_version < 18 {
            info!("Running migration 18: Session recovery state");
            tx.execute_batch(
                r#"
                ALTER TABLE sessions ADD COLUMN recovery_json TEXT;
                "#,
            )?;
            self.set_schema_version_tx(&tx, 18)?;
        }

        // Migration 19: Structured runtime traces
        if current_version < 19 {
            info!("Running migration 19: Runtime traces");
            tx.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS runtime_traces (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    session_id TEXT NOT NULL,
                    run_id TEXT NOT NULL,
                    sequence INTEGER NOT NULL,
                    turn INTEGER NOT NULL,
                    event_type TEXT NOT NULL,
                    payload_json TEXT NOT NULL,
                    failure_category TEXT,
                    stop_reason TEXT,
                    created_at TEXT NOT NULL,
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
                    UNIQUE(session_id, sequence)
                );

                CREATE INDEX IF NOT EXISTS idx_runtime_traces_session_sequence
                    ON runtime_traces(session_id, sequence);

                CREATE INDEX IF NOT EXISTS idx_runtime_traces_session_run
                    ON runtime_traces(session_id, run_id, sequence);
                "#,
            )?;
            self.set_schema_version_tx(&tx, 19)?;
        }

        // Migration 20: Autonomous tasks for Mako agent coordination
        if current_version < 20 {
            info!("Running migration 20: Autonomous tasks");
            tx.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS autonomous_tasks (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL,
                    subject TEXT NOT NULL,
                    description TEXT NOT NULL DEFAULT '',
                    status TEXT NOT NULL DEFAULT 'pending',
                    owner TEXT,
                    blocked_by TEXT DEFAULT '[]',
                    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    completed_at TEXT,
                    result TEXT,
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
                );

                CREATE INDEX IF NOT EXISTS idx_autonomous_tasks_session
                    ON autonomous_tasks(session_id);
                CREATE INDEX IF NOT EXISTS idx_autonomous_tasks_status
                    ON autonomous_tasks(session_id, status);
                "#,
            )?;
            self.set_schema_version_tx(&tx, 20)?;
        }

        // Migration 21: Research reports
        if current_version < 21 {
            info!("Running migration 21: Reports");
            tx.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS reports (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    session_id TEXT NOT NULL,
                    project_dir TEXT,
                    content TEXT NOT NULL,
                    summary TEXT NOT NULL DEFAULT '',
                    tags TEXT NOT NULL DEFAULT '[]',
                    sources TEXT NOT NULL DEFAULT '[]',
                    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                );

                CREATE INDEX IF NOT EXISTS idx_reports_session
                    ON reports(session_id);
                CREATE INDEX IF NOT EXISTS idx_reports_project
                    ON reports(project_dir);
                "#,
            )?;
            self.set_schema_version_tx(&tx, 21)?;
        }

        // Migration 22: Explicit workspace context on sessions
        if current_version < 22 {
            info!("Running migration 22: Session workspace context");
            if !Self::column_exists(&tx, "sessions", "project_dir") {
                tx.execute_batch("ALTER TABLE sessions ADD COLUMN project_dir TEXT;")?;
            }
            if !Self::column_exists(&tx, "sessions", "workspace_mode") {
                tx.execute_batch(
                    r#"ALTER TABLE sessions ADD COLUMN workspace_mode TEXT NOT NULL DEFAULT 'neutral'
                        CHECK (workspace_mode IN ('neutral', 'selected', 'created'));"#,
                )?;
            }
            tx.execute_batch(
                r#"

                UPDATE sessions
                   SET project_dir = COALESCE(project_dir, working_dir),
                       workspace_mode = CASE
                           WHEN COALESCE(project_dir, working_dir) IS NULL THEN 'neutral'
                           ELSE 'selected'
                       END;

                CREATE INDEX IF NOT EXISTS idx_sessions_project_dir
                    ON sessions(project_dir);
                CREATE INDEX IF NOT EXISTS idx_sessions_workspace_mode
                    ON sessions(workspace_mode);
                "#,
            )?;
            self.set_schema_version_tx(&tx, 22)?;
        }

        // Migration 23: First-class delegated run persistence
        if current_version < 23 {
            info!("Running migration 23: Delegated runs");
            tx.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS delegated_runs (
                    delegated_run_id TEXT PRIMARY KEY,
                    parent_session_id TEXT NOT NULL,
                    parent_tool_call_id TEXT,
                    role TEXT NOT NULL
                        CHECK (role IN ('explore', 'build', 'planner', 'verifier')),
                    stage TEXT NOT NULL
                        CHECK (stage IN ('created', 'running', 'synthesizing', 'complete', 'degraded', 'failed', 'cancelled')),
                    provider TEXT,
                    model TEXT,
                    resumable INTEGER NOT NULL DEFAULT 0,
                    resumed_from_run_id TEXT,
                    target_scope_key TEXT NOT NULL,
                    target_scope_json TEXT NOT NULL,
                    snapshot_json TEXT,
                    artifact_json TEXT,
                    human_review TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    completed_at TEXT,
                    FOREIGN KEY (parent_session_id) REFERENCES sessions(id) ON DELETE CASCADE
                );

                CREATE INDEX IF NOT EXISTS idx_delegated_runs_session_updated
                    ON delegated_runs(parent_session_id, updated_at DESC);
                CREATE INDEX IF NOT EXISTS idx_delegated_runs_session_scope
                    ON delegated_runs(parent_session_id, role, target_scope_key, updated_at DESC);
                CREATE INDEX IF NOT EXISTS idx_delegated_runs_parent_tool
                    ON delegated_runs(parent_tool_call_id);
                "#,
            )?;
            self.set_schema_version_tx(&tx, 23)?;
        }

        // Migration 24: First-class session types
        if current_version < 24 {
            info!("Running migration 24: Session types");
            if current_version > 0 && !Self::column_exists(&tx, "sessions", "session_type") {
                tx.execute_batch(
                    r#"
                    ALTER TABLE sessions ADD COLUMN session_type TEXT NOT NULL DEFAULT 'code'
                        CHECK (session_type IN ('chat', 'code', 'mako'));
                    "#,
                )?;
            }
            tx.execute_batch(
                r#"
                CREATE INDEX IF NOT EXISTS idx_sessions_session_type
                    ON sessions(session_type);
                "#,
            )?;
            self.set_schema_version_tx(&tx, 24)?;
        }

        // Migration 25: Autonomous tasks + reports tables (datetime defaults)
        if current_version < 25 {
            info!("Running migration 25: autonomous_tasks + reports tables");
            tx.execute_batch(
                "CREATE TABLE IF NOT EXISTS autonomous_tasks (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL,
                    subject TEXT NOT NULL,
                    description TEXT NOT NULL DEFAULT '',
                    status TEXT NOT NULL DEFAULT 'pending',
                    owner TEXT,
                    blocked_by TEXT DEFAULT '[]',
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                    completed_at TEXT,
                    result TEXT,
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
                );
                CREATE INDEX IF NOT EXISTS idx_autonomous_tasks_session ON autonomous_tasks(session_id);
                CREATE INDEX IF NOT EXISTS idx_autonomous_tasks_status ON autonomous_tasks(session_id, status);

                CREATE TABLE IF NOT EXISTS reports (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    session_id TEXT NOT NULL,
                    project_dir TEXT,
                    content TEXT NOT NULL,
                    summary TEXT NOT NULL DEFAULT '',
                    tags TEXT NOT NULL DEFAULT '[]',
                    sources TEXT NOT NULL DEFAULT '[]',
                    created_at TEXT NOT NULL DEFAULT (datetime('now'))
                );
                CREATE INDEX IF NOT EXISTS idx_reports_session ON reports(session_id);
                CREATE INDEX IF NOT EXISTS idx_reports_project ON reports(project_dir);",
            )
            .context("Migration 25: autonomous_tasks + reports tables")?;
            self.set_schema_version_tx(&tx, 25)?;
        }

        // Migration 26: APNs device tokens for iOS push notifications
        if current_version < 26 {
            info!("Running migration 26: APNs device tokens");
            tx.execute_batch(
                "CREATE TABLE IF NOT EXISTS apns_devices (
                    id TEXT PRIMARY KEY,
                    user_id TEXT,
                    device_token TEXT NOT NULL UNIQUE,
                    bundle_id TEXT NOT NULL DEFAULT 'io.krusty.mobile',
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    last_used_at TEXT,
                    last_success_at TEXT,
                    last_failure_at TEXT,
                    last_failure_reason TEXT,
                    failure_count INTEGER NOT NULL DEFAULT 0
                );
                CREATE INDEX IF NOT EXISTS idx_apns_devices_user ON apns_devices(user_id);",
            )
            .context("Migration 26: APNs device tokens")?;
            self.set_schema_version_tx(&tx, 26)?;
        }

        // Migration 27: Persisted Mako runtime state
        if current_version < 27 {
            info!("Running migration 27: Mako runtime state");
            tx.execute_batch(
                "CREATE TABLE IF NOT EXISTS mako_runtime_state (
                    session_id TEXT PRIMARY KEY,
                    status TEXT NOT NULL
                        CHECK (status IN ('idle', 'running', 'sleeping', 'awaiting_input', 'paused', 'error', 'cancelled')),
                    next_wake_at TEXT,
                    sleep_reason TEXT,
                    last_error TEXT,
                    current_run_id TEXT,
                    last_wake_reason TEXT,
                    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
                );
                CREATE INDEX IF NOT EXISTS idx_mako_runtime_state_status
                    ON mako_runtime_state(status);
                CREATE INDEX IF NOT EXISTS idx_mako_runtime_state_next_wake
                    ON mako_runtime_state(next_wake_at);",
            )
            .context("Migration 27: Mako runtime state")?;
            self.set_schema_version_tx(&tx, 27)?;
        }

        // Migration 28: Persisted Mako run priority
        if current_version < 28 {
            info!("Running migration 28: Mako run priority");
            if !Self::column_exists(&tx, "mako_runtime_state", "priority") {
                tx.execute_batch(
                    "ALTER TABLE mako_runtime_state
                     ADD COLUMN priority TEXT NOT NULL DEFAULT 'normal';",
                )?;
            }
            self.set_schema_version_tx(&tx, 28)?;
        }

        // Migration 29: Persisted Mako crew assignment
        if current_version < 29 {
            info!("Running migration 29: Mako crew assignment");
            if !Self::column_exists(&tx, "mako_runtime_state", "crew_slug") {
                tx.execute_batch(
                    "ALTER TABLE mako_runtime_state
                     ADD COLUMN crew_slug TEXT;",
                )?;
            }
            self.set_schema_version_tx(&tx, 29)?;
        }

        // Migration 30: Persisted Mako attention item state
        if current_version < 30 {
            info!("Running migration 30: Mako attention state");
            tx.execute_batch(
                "CREATE TABLE IF NOT EXISTS mako_attention_state (
                    user_scope TEXT NOT NULL DEFAULT '',
                    item_id TEXT NOT NULL,
                    read INTEGER NOT NULL DEFAULT 0,
                    cleared INTEGER NOT NULL DEFAULT 0,
                    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                    PRIMARY KEY (user_scope, item_id)
                );
                CREATE INDEX IF NOT EXISTS idx_mako_attention_state_user_scope
                    ON mako_attention_state(user_scope);",
            )
            .context("Migration 30: Mako attention state")?;
            self.set_schema_version_tx(&tx, 30)?;
        }

        // Migration 31: Persist session permission mode
        if current_version < 31 {
            info!("Running migration 31: Session permission mode");
            if !Self::column_exists(&tx, "sessions", "permission_mode") {
                tx.execute_batch(
                    "ALTER TABLE sessions
                     ADD COLUMN permission_mode TEXT NOT NULL DEFAULT 'autonomous'
                     CHECK (permission_mode IN ('supervised', 'autonomous'));",
                )?;
            }
            self.set_schema_version_tx(&tx, 31)?;
        }

        // Migration 32: Compaction checkpoints and transcript segments
        if current_version < 32 {
            info!("Running migration 32: Compaction checkpoints and segments");
            tx.execute_batch(
                "CREATE TABLE IF NOT EXISTS compaction_checkpoints (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL,
                    prompt_index_at_compaction INTEGER NOT NULL,
                    pre_compact_message_ids_json TEXT NOT NULL,
                    compacted_history_json TEXT NOT NULL,
                    original_user_info TEXT,
                    reread_file_paths_json TEXT NOT NULL DEFAULT '[]',
                    schema_version INTEGER NOT NULL DEFAULT 1,
                    created_at TEXT NOT NULL,
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
                );
                CREATE INDEX IF NOT EXISTS idx_compaction_checkpoints_session
                    ON compaction_checkpoints(session_id);
                CREATE TABLE IF NOT EXISTS compaction_segments (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL,
                    checkpoint_id TEXT NOT NULL,
                    message_id_start INTEGER NOT NULL,
                    message_id_end INTEGER NOT NULL,
                    segment_markdown TEXT NOT NULL,
                    token_estimate INTEGER NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL,
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
                    FOREIGN KEY (checkpoint_id) REFERENCES compaction_checkpoints(id) ON DELETE CASCADE
                );
                CREATE INDEX IF NOT EXISTS idx_compaction_segments_session
                    ON compaction_segments(session_id);
                CREATE INDEX IF NOT EXISTS idx_compaction_segments_checkpoint
                    ON compaction_segments(checkpoint_id);",
            )
            .context("Migration 32: Compaction checkpoints and segments")?;
            self.set_schema_version_tx(&tx, 32)?;
        }

        // Migration 33: Remove legacy compaction memory leaks and duplicate checkpoint history.
        if current_version < 33 {
            info!("Running migration 33: Compaction memory cleanup");
            if Self::table_exists(&tx, "agent_memories") {
                tx.execute(
                    "DELETE FROM agent_memories
                     WHERE memory_type = 'project'
                       AND title LIKE 'Compaction flush #%';",
                    [],
                )
                .context("Migration 33: delete legacy compaction flush memories")?;
            }
            tx.execute(
                "UPDATE compaction_checkpoints
                 SET compacted_history_json = '[]'
                 WHERE compacted_history_json <> '[]';",
                [],
            )
            .context("Migration 33: redact duplicate checkpoint history")?;
            self.set_schema_version_tx(&tx, 33)?;
        }

        // Migration 34: Queryable provider-call classification.
        if current_version < 34 {
            info!("Running migration 34: Provider-call trace classification");
            if Self::table_exists(&tx, "runtime_traces") {
                if !Self::column_exists(&tx, "runtime_traces", "call_kind") {
                    tx.execute_batch("ALTER TABLE runtime_traces ADD COLUMN call_kind TEXT;")?;
                }
                if !Self::column_exists(&tx, "runtime_traces", "operation") {
                    tx.execute_batch("ALTER TABLE runtime_traces ADD COLUMN operation TEXT;")?;
                }
                tx.execute_batch(
                    "UPDATE runtime_traces
                     SET call_kind = json_extract(payload_json, '$.call_kind')
                     WHERE call_kind IS NULL
                       AND event_type = 'provider_call';
                     UPDATE runtime_traces
                     SET operation = json_extract(payload_json, '$.operation')
                     WHERE operation IS NULL
                       AND event_type = 'provider_call';
                     CREATE INDEX IF NOT EXISTS idx_runtime_traces_provider_operation
                       ON runtime_traces(session_id, call_kind, operation, sequence);",
                )
                .context("Migration 34: classify provider-call traces")?;
            }
            self.set_schema_version_tx(&tx, 34)?;
        }

        // Migration 35: Database-owned, revisioned Mako identity profiles.
        if current_version < 35 {
            info!("Running migration 35: Mako identity profiles");
            tx.execute_batch(
                r#"
                CREATE TABLE mako_profiles (
                    id TEXT PRIMARY KEY,
                    user_id TEXT,
                    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                );
                CREATE UNIQUE INDEX idx_mako_profiles_user
                    ON mako_profiles(user_id) WHERE user_id IS NOT NULL;

                CREATE TABLE mako_profile_documents (
                    profile_id TEXT NOT NULL,
                    kind TEXT NOT NULL
                        CHECK (kind IN ('soul', 'identity', 'user', 'heartbeat', 'channels')),
                    content TEXT NOT NULL,
                    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                    PRIMARY KEY (profile_id, kind),
                    FOREIGN KEY (profile_id) REFERENCES mako_profiles(id) ON DELETE CASCADE
                );

                CREATE TABLE mako_crew_profiles (
                    profile_id TEXT NOT NULL,
                    slug TEXT NOT NULL,
                    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                    PRIMARY KEY (profile_id, slug),
                    FOREIGN KEY (profile_id) REFERENCES mako_profiles(id) ON DELETE CASCADE
                );

                CREATE TABLE mako_crew_documents (
                    profile_id TEXT NOT NULL,
                    slug TEXT NOT NULL,
                    kind TEXT NOT NULL CHECK (kind IN ('identity', 'soul')),
                    content TEXT NOT NULL,
                    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                    PRIMARY KEY (profile_id, slug, kind),
                    FOREIGN KEY (profile_id, slug)
                        REFERENCES mako_crew_profiles(profile_id, slug) ON DELETE CASCADE
                );
                "#,
            )
            .context("Migration 35: Mako identity profiles")?;
            self.set_schema_version_tx(&tx, 35)?;
        }

        // Migration 36: Durable Mako controllers, schedules, runs, leases, and event journal.
        if current_version < 36 {
            info!("Running migration 36: Durable Mako scheduler");
            tx.execute_batch(
                r#"
                CREATE TABLE mako_controllers (
                    id TEXT PRIMARY KEY,
                    scope_key TEXT NOT NULL UNIQUE,
                    user_id TEXT,
                    session_id TEXT NOT NULL UNIQUE,
                    status TEXT NOT NULL CHECK (status IN ('active', 'paused', 'disabled')),
                    timezone TEXT NOT NULL,
                    max_concurrent_runs INTEGER NOT NULL CHECK (max_concurrent_runs > 0),
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
                );
                CREATE INDEX idx_mako_controllers_user ON mako_controllers(user_id);
                CREATE INDEX idx_mako_controllers_status ON mako_controllers(status);

                CREATE TABLE mako_schedules (
                    id TEXT PRIMARY KEY,
                    controller_id TEXT NOT NULL,
                    title TEXT NOT NULL,
                    summary TEXT NOT NULL,
                    objective TEXT NOT NULL,
                    recurrence_kind TEXT NOT NULL,
                    recurrence_json TEXT NOT NULL,
                    timezone TEXT NOT NULL,
                    gap_policy TEXT NOT NULL CHECK (gap_policy IN ('shift_forward', 'skip')),
                    fold_policy TEXT NOT NULL CHECK (fold_policy IN ('first', 'second')),
                    next_fire_at TEXT,
                    last_scheduled_for TEXT,
                    status TEXT NOT NULL
                        CHECK (status IN ('enabled', 'paused', 'completed', 'cancelled')),
                    priority INTEGER NOT NULL DEFAULT 0,
                    project_dir TEXT,
                    model TEXT,
                    crew_slug TEXT,
                    misfire_policy TEXT NOT NULL
                        CHECK (misfire_policy IN ('fire_once', 'skip', 'catch_up')),
                    misfire_grace_secs INTEGER NOT NULL CHECK (misfire_grace_secs >= 0),
                    catch_up_limit INTEGER NOT NULL CHECK (catch_up_limit >= 0),
                    overlap_policy TEXT NOT NULL CHECK (overlap_policy IN ('skip', 'queue_one', 'allow')),
                    max_attempts INTEGER NOT NULL CHECK (max_attempts > 0),
                    retry_base_secs INTEGER NOT NULL CHECK (retry_base_secs >= 0),
                    retry_max_secs INTEGER NOT NULL CHECK (retry_max_secs >= 0),
                    retry_jitter TEXT NOT NULL CHECK (retry_jitter IN ('none', 'full')),
                    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
                    created_by TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    FOREIGN KEY (controller_id) REFERENCES mako_controllers(id) ON DELETE CASCADE
                );
                CREATE INDEX idx_mako_schedules_due
                    ON mako_schedules(status, next_fire_at);
                CREATE INDEX idx_mako_schedules_controller
                    ON mako_schedules(controller_id, status);

                CREATE TABLE mako_schedule_occurrences (
                    id TEXT PRIMARY KEY,
                    schedule_id TEXT NOT NULL,
                    scheduled_for TEXT NOT NULL,
                    run_id TEXT,
                    status TEXT NOT NULL
                        CHECK (status IN ('pending', 'queued', 'skipped', 'coalesced', 'running', 'succeeded', 'failed', 'cancelled')),
                    decision_reason TEXT,
                    coalesced_count INTEGER NOT NULL DEFAULT 0 CHECK (coalesced_count >= 0),
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    UNIQUE (schedule_id, scheduled_for),
                    FOREIGN KEY (schedule_id) REFERENCES mako_schedules(id) ON DELETE CASCADE
                );

                CREATE TABLE mako_runs (
                    id TEXT PRIMARY KEY,
                    controller_id TEXT NOT NULL,
                    session_id TEXT,
                    schedule_id TEXT,
                    occurrence_id TEXT,
                    kind TEXT NOT NULL
                        CHECK (kind IN ('dispatch', 'scheduled', 'controller_child', 'legacy_resume')),
                    objective TEXT NOT NULL,
                    config_json TEXT NOT NULL,
                    status TEXT NOT NULL
                        CHECK (status IN ('queued', 'leased', 'running', 'sleeping', 'retry_wait', 'awaiting_input', 'recovery_required', 'succeeded', 'failed', 'cancelled', 'dead_letter')),
                    priority INTEGER NOT NULL DEFAULT 0,
                    concurrency_key TEXT,
                    scheduled_for TEXT,
                    available_at TEXT NOT NULL,
                    wake_at TEXT,
                    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
                    max_attempts INTEGER NOT NULL CHECK (max_attempts > 0),
                    lease_owner TEXT,
                    lease_token TEXT,
                    lease_epoch INTEGER CHECK (lease_epoch IS NULL OR lease_epoch >= 0),
                    lease_expires_at TEXT,
                    heartbeat_at TEXT,
                    last_stop_reason TEXT,
                    last_error TEXT,
                    outcome_json TEXT,
                    created_at TEXT NOT NULL,
                    started_at TEXT,
                    finished_at TEXT,
                    updated_at TEXT NOT NULL,
                    FOREIGN KEY (controller_id) REFERENCES mako_controllers(id) ON DELETE CASCADE,
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL,
                    FOREIGN KEY (schedule_id) REFERENCES mako_schedules(id) ON DELETE SET NULL,
                    FOREIGN KEY (occurrence_id) REFERENCES mako_schedule_occurrences(id) ON DELETE SET NULL
                );
                CREATE INDEX idx_mako_runs_claim
                    ON mako_runs(status, available_at, priority DESC, created_at);
                CREATE INDEX idx_mako_runs_controller_status
                    ON mako_runs(controller_id, status);
                CREATE INDEX idx_mako_runs_concurrency
                    ON mako_runs(concurrency_key, status) WHERE concurrency_key IS NOT NULL;
                CREATE INDEX idx_mako_runs_lease_expiry
                    ON mako_runs(lease_expires_at) WHERE lease_expires_at IS NOT NULL;
                CREATE INDEX idx_mako_runs_session ON mako_runs(session_id);

                CREATE TABLE mako_run_attempts (
                    id TEXT PRIMARY KEY,
                    run_id TEXT NOT NULL,
                    attempt_no INTEGER NOT NULL CHECK (attempt_no > 0),
                    worker_id TEXT NOT NULL,
                    lease_token TEXT NOT NULL,
                    lease_epoch INTEGER NOT NULL CHECK (lease_epoch >= 0),
                    started_at TEXT NOT NULL,
                    finished_at TEXT,
                    outcome TEXT NOT NULL
                        CHECK (outcome IN ('leased', 'succeeded', 'failed', 'retry_scheduled', 'sleeping', 'awaiting_input', 'recovery_required', 'cancelled', 'abandoned', 'dead_letter')),
                    stop_reason TEXT,
                    error TEXT,
                    retry_at TEXT,
                    trace_sequence_start INTEGER,
                    trace_sequence_end INTEGER,
                    UNIQUE (run_id, attempt_no),
                    FOREIGN KEY (run_id) REFERENCES mako_runs(id) ON DELETE CASCADE
                );
                CREATE INDEX idx_mako_run_attempts_run
                    ON mako_run_attempts(run_id, attempt_no);

                CREATE TABLE mako_daemon_leases (
                    lease_name TEXT PRIMARY KEY,
                    owner_id TEXT NOT NULL,
                    fencing_token INTEGER NOT NULL CHECK (fencing_token >= 0),
                    acquired_at TEXT NOT NULL,
                    heartbeat_at TEXT NOT NULL,
                    expires_at TEXT NOT NULL
                );

                CREATE TABLE mako_idempotency_keys (
                    scope_key TEXT NOT NULL,
                    operation TEXT NOT NULL,
                    idempotency_key TEXT NOT NULL,
                    request_hash TEXT NOT NULL,
                    resource_id TEXT,
                    response_json TEXT,
                    created_at TEXT NOT NULL,
                    expires_at TEXT NOT NULL,
                    PRIMARY KEY (scope_key, operation, idempotency_key)
                );
                CREATE INDEX idx_mako_idempotency_expiry ON mako_idempotency_keys(expires_at);

                CREATE TABLE mako_controller_events (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    controller_id TEXT NOT NULL,
                    sequence INTEGER NOT NULL CHECK (sequence > 0),
                    event_type TEXT NOT NULL,
                    run_id TEXT,
                    schedule_id TEXT,
                    dedupe_key TEXT,
                    payload_json TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    UNIQUE (controller_id, sequence),
                    FOREIGN KEY (controller_id) REFERENCES mako_controllers(id) ON DELETE CASCADE,
                    FOREIGN KEY (run_id) REFERENCES mako_runs(id) ON DELETE SET NULL,
                    FOREIGN KEY (schedule_id) REFERENCES mako_schedules(id) ON DELETE SET NULL
                );
                CREATE UNIQUE INDEX idx_mako_controller_events_dedupe
                    ON mako_controller_events(controller_id, dedupe_key)
                    WHERE dedupe_key IS NOT NULL;
                CREATE INDEX idx_mako_controller_events_replay
                    ON mako_controller_events(controller_id, sequence);
                "#,
            )
            .context("Migration 36: Durable Mako scheduler")?;
            self.set_schema_version_tx(&tx, 36)?;
        }

        // Migration 37: Owned cross-session episodic recall with a bounded FTS index.
        if current_version < 37 {
            info!("Running migration 37: Mako episodic recall");
            tx.execute_batch(
                r#"
                CREATE TABLE conversation_episodes (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    session_id TEXT NOT NULL,
                    source_message_id INTEGER NOT NULL,
                    role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
                    body TEXT NOT NULL,
                    content_hash TEXT NOT NULL,
                    occurred_at TEXT NOT NULL,
                    UNIQUE (session_id, source_message_id),
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
                    FOREIGN KEY (source_message_id) REFERENCES messages(id) ON DELETE CASCADE
                );
                CREATE INDEX idx_conversation_episodes_session_time
                    ON conversation_episodes(session_id, occurred_at DESC);
                CREATE INDEX idx_conversation_episodes_hash
                    ON conversation_episodes(content_hash);

                CREATE VIRTUAL TABLE conversation_episodes_fts USING fts5(
                    body,
                    content = 'conversation_episodes',
                    content_rowid = 'id',
                    tokenize = 'porter unicode61'
                );
                CREATE TRIGGER conversation_episodes_ai AFTER INSERT ON conversation_episodes BEGIN
                    INSERT INTO conversation_episodes_fts(rowid, body) VALUES (new.id, new.body);
                END;
                CREATE TRIGGER conversation_episodes_ad AFTER DELETE ON conversation_episodes BEGIN
                    INSERT INTO conversation_episodes_fts(conversation_episodes_fts, rowid, body)
                    VALUES ('delete', old.id, old.body);
                END;
                CREATE TRIGGER conversation_episodes_au AFTER UPDATE ON conversation_episodes BEGIN
                    INSERT INTO conversation_episodes_fts(conversation_episodes_fts, rowid, body)
                    VALUES ('delete', old.id, old.body);
                    INSERT INTO conversation_episodes_fts(rowid, body) VALUES (new.id, new.body);
                END;
                "#,
            )
            .context("Migration 37: Mako episodic recall")?;
            self.set_schema_version_tx(&tx, 37)?;
        }

        // Migration 38: Governed post-turn learning proposals and reviewer checkpoints.
        if current_version < 38 {
            info!("Running migration 38: Governed Mako learning");
            tx.execute_batch(
                r#"
                CREATE TABLE mako_learning_runs (
                    session_id TEXT NOT NULL,
                    through_message_id INTEGER NOT NULL,
                    status TEXT NOT NULL CHECK (status IN ('running', 'completed', 'failed')),
                    model TEXT,
                    created_at TEXT NOT NULL,
                    completed_at TEXT,
                    PRIMARY KEY (session_id, through_message_id),
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
                    FOREIGN KEY (through_message_id) REFERENCES messages(id) ON DELETE CASCADE
                );

                CREATE TABLE mako_learning_candidates (
                    id TEXT PRIMARY KEY,
                    user_id TEXT,
                    project_dir TEXT,
                    canonical_key TEXT NOT NULL,
                    kind TEXT NOT NULL
                        CHECK (kind IN ('user_preference', 'user_correction', 'project_fact', 'procedure', 'relationship_context', 'forget')),
                    proposed_content TEXT NOT NULL,
                    evidence_session_id TEXT NOT NULL,
                    evidence_message_id INTEGER NOT NULL,
                    evidence_excerpt TEXT NOT NULL,
                    explicit INTEGER NOT NULL CHECK (explicit IN (0, 1)),
                    confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                    sensitivity TEXT NOT NULL CHECK (sensitivity IN ('normal', 'sensitive', 'prohibited')),
                    status TEXT NOT NULL
                        CHECK (status IN ('pending', 'accepted', 'auto_accepted', 'rejected', 'tombstoned')),
                    reason TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    reviewed_at TEXT,
                    UNIQUE (evidence_session_id, evidence_message_id, canonical_key),
                    FOREIGN KEY (evidence_session_id) REFERENCES sessions(id) ON DELETE CASCADE,
                    FOREIGN KEY (evidence_message_id) REFERENCES messages(id) ON DELETE CASCADE
                );
                CREATE INDEX idx_mako_learning_candidates_owner_status
                    ON mako_learning_candidates(user_id, status, created_at DESC);
                CREATE INDEX idx_mako_learning_candidates_project
                    ON mako_learning_candidates(project_dir, status);
                "#,
            )
            .context("Migration 38: Governed Mako learning")?;
            self.set_schema_version_tx(&tx, 38)?;
        }

        // Migration 39: Canonical, provenance-aware memory and derived knowledge snapshots.
        if current_version < 39 {
            info!("Running migration 39: Canonical Mako memory");

            // Memory storage historically initialized lazily. Creating the
            // legacy columns first keeps this migration valid for databases
            // that have never opened the memory API.
            tx.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS agent_memories (
                    id TEXT PRIMARY KEY,
                    memory_type TEXT NOT NULL
                        CHECK (memory_type IN ('user', 'feedback', 'project', 'reference')),
                    title TEXT NOT NULL,
                    content TEXT NOT NULL,
                    project_dir TEXT,
                    user_id TEXT,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                    canonical_key TEXT,
                    namespace TEXT NOT NULL DEFAULT 'shared'
                        CHECK (namespace IN ('shared', 'mako', 'crew')),
                    namespace_id TEXT,
                    status TEXT NOT NULL DEFAULT 'active'
                        CHECK (status IN ('active', 'superseded', 'deleted')),
                    source TEXT NOT NULL DEFAULT 'legacy'
                        CHECK (source IN ('legacy', 'user', 'agent', 'tool', 'import', 'compaction', 'system')),
                    source_session_id TEXT,
                    source_message_id TEXT,
                    confidence REAL NOT NULL DEFAULT 1.0
                        CHECK (confidence >= 0.0 AND confidence <= 1.0),
                    sensitivity TEXT NOT NULL DEFAULT 'normal'
                        CHECK (sensitivity IN ('normal', 'sensitive', 'secret')),
                    pinned INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)),
                    supersedes_id TEXT REFERENCES agent_memories(id),
                    last_accessed_at TEXT,
                    access_count INTEGER NOT NULL DEFAULT 0 CHECK (access_count >= 0)
                );
                "#,
            )
            .context("Migration 39: ensure memory base table")?;

            if !Self::column_exists(&tx, "agent_memories", "canonical_key") {
                tx.execute_batch("ALTER TABLE agent_memories ADD COLUMN canonical_key TEXT;")?;
            }
            if !Self::column_exists(&tx, "agent_memories", "namespace") {
                tx.execute_batch(
                    "ALTER TABLE agent_memories ADD COLUMN namespace TEXT NOT NULL DEFAULT 'shared' CHECK (namespace IN ('shared', 'mako', 'crew'));",
                )?;
            }
            if !Self::column_exists(&tx, "agent_memories", "namespace_id") {
                tx.execute_batch("ALTER TABLE agent_memories ADD COLUMN namespace_id TEXT;")?;
            }
            if !Self::column_exists(&tx, "agent_memories", "status") {
                tx.execute_batch(
                    "ALTER TABLE agent_memories ADD COLUMN status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'superseded', 'deleted'));",
                )?;
            }
            if !Self::column_exists(&tx, "agent_memories", "source") {
                tx.execute_batch(
                    "ALTER TABLE agent_memories ADD COLUMN source TEXT NOT NULL DEFAULT 'legacy' CHECK (source IN ('legacy', 'user', 'agent', 'tool', 'import', 'compaction', 'system'));",
                )?;
            }
            if !Self::column_exists(&tx, "agent_memories", "source_session_id") {
                tx.execute_batch("ALTER TABLE agent_memories ADD COLUMN source_session_id TEXT;")?;
            }
            if !Self::column_exists(&tx, "agent_memories", "source_message_id") {
                tx.execute_batch("ALTER TABLE agent_memories ADD COLUMN source_message_id TEXT;")?;
            }
            if !Self::column_exists(&tx, "agent_memories", "confidence") {
                tx.execute_batch(
                    "ALTER TABLE agent_memories ADD COLUMN confidence REAL NOT NULL DEFAULT 1.0 CHECK (confidence >= 0.0 AND confidence <= 1.0);",
                )?;
            }
            if !Self::column_exists(&tx, "agent_memories", "sensitivity") {
                tx.execute_batch(
                    "ALTER TABLE agent_memories ADD COLUMN sensitivity TEXT NOT NULL DEFAULT 'normal' CHECK (sensitivity IN ('normal', 'sensitive', 'secret'));",
                )?;
            }
            if !Self::column_exists(&tx, "agent_memories", "pinned") {
                tx.execute_batch(
                    "ALTER TABLE agent_memories ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1));",
                )?;
            }
            if !Self::column_exists(&tx, "agent_memories", "supersedes_id") {
                tx.execute_batch(
                    "ALTER TABLE agent_memories ADD COLUMN supersedes_id TEXT REFERENCES agent_memories(id);",
                )?;
            }
            if !Self::column_exists(&tx, "agent_memories", "last_accessed_at") {
                tx.execute_batch("ALTER TABLE agent_memories ADD COLUMN last_accessed_at TEXT;")?;
            }
            if !Self::column_exists(&tx, "agent_memories", "access_count") {
                tx.execute_batch(
                    "ALTER TABLE agent_memories ADD COLUMN access_count INTEGER NOT NULL DEFAULT 0 CHECK (access_count >= 0);",
                )?;
            }

            tx.execute_batch(
                r#"
                CREATE INDEX IF NOT EXISTS idx_agent_memories_type
                    ON agent_memories(memory_type);
                CREATE INDEX IF NOT EXISTS idx_agent_memories_project
                    ON agent_memories(project_dir);
                CREATE INDEX IF NOT EXISTS idx_agent_memories_user
                    ON agent_memories(user_id);
                CREATE INDEX idx_agent_memories_active_scope
                    ON agent_memories(status, user_id, project_dir, namespace, namespace_id);
                CREATE INDEX idx_agent_memories_canonical_key
                    ON agent_memories(canonical_key, status);
                CREATE UNIQUE INDEX idx_agent_memories_active_canonical
                    ON agent_memories(
                        COALESCE(user_id, ''), COALESCE(project_dir, ''), namespace,
                        COALESCE(namespace_id, ''), canonical_key
                    )
                    WHERE status = 'active' AND canonical_key IS NOT NULL;

                CREATE TABLE agent_memory_revisions (
                    id TEXT PRIMARY KEY,
                    memory_id TEXT NOT NULL,
                    revision INTEGER NOT NULL CHECK (revision > 0),
                    event TEXT NOT NULL
                        CHECK (event IN ('created', 'updated', 'superseded', 'deleted')),
                    snapshot_json TEXT NOT NULL,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    UNIQUE (memory_id, revision),
                    FOREIGN KEY (memory_id) REFERENCES agent_memories(id) ON DELETE CASCADE
                );
                CREATE INDEX idx_agent_memory_revisions_memory
                    ON agent_memory_revisions(memory_id, revision);

                INSERT OR IGNORE INTO agent_memory_revisions
                    (id, memory_id, revision, event, snapshot_json, created_at)
                SELECT
                    'migration39:' || id,
                    id,
                    1,
                    'created',
                    json_object(
                        'id', id,
                        'memory_type', memory_type,
                        'title', title,
                        'content', content,
                        'project_dir', project_dir,
                        'user_id', user_id,
                        'created_at', created_at,
                        'updated_at', updated_at,
                        'canonical_key', canonical_key,
                        'namespace', namespace,
                        'namespace_id', namespace_id,
                        'status', status,
                        'source', source,
                        'source_session_id', source_session_id,
                        'source_message_id', source_message_id,
                        'confidence', confidence,
                        'sensitivity', sensitivity,
                        'pinned', CASE pinned WHEN 0 THEN json('false') ELSE json('true') END,
                        'supersedes_id', supersedes_id,
                        'last_accessed_at', last_accessed_at,
                        'access_count', access_count
                    ),
                    created_at
                FROM agent_memories;

                CREATE TABLE knowledge_snapshots (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    content TEXT NOT NULL,
                    project_dir TEXT,
                    user_id TEXT,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                );
                CREATE UNIQUE INDEX idx_knowledge_snapshots_exact_scope
                    ON knowledge_snapshots(COALESCE(project_dir, ''), COALESCE(user_id, ''));

                INSERT OR IGNORE INTO knowledge_snapshots
                    (id, title, content, project_dir, user_id, created_at, updated_at)
                SELECT id, title, content, project_dir, user_id, created_at, updated_at
                FROM agent_memories
                WHERE memory_type = 'project' AND title = 'Current Snapshot';
                DELETE FROM agent_memories
                WHERE memory_type = 'project' AND title = 'Current Snapshot';
                "#,
            )
            .context("Migration 39: canonical memory and knowledge tables")?;

            self.set_schema_version_tx(&tx, 39)?;
        }

        // Migration 40: Make every durable Mako run transition produce an
        // authoritative replay event in the same SQLite transaction. Runtime
        // publishers may crash after commit; subscribers can still recover the
        // event from this journal.
        if current_version < 40 {
            info!("Running migration 40: Atomic Mako run transition journal");
            // Some legacy/specialized databases advance the shared schema
            // version without materializing the optional Mako tables. Keep
            // their upgrade valid; any database with the Mako contract gets
            // the trigger atomically with the version bump.
            if Self::table_exists(&tx, "mako_runs")
                && Self::table_exists(&tx, "mako_controller_events")
            {
                tx.execute_batch(
                    r#"
                CREATE TRIGGER mako_runs_transition_event
                AFTER UPDATE OF status ON mako_runs
                WHEN OLD.status <> NEW.status
                BEGIN
                    INSERT OR IGNORE INTO mako_controller_events (
                        controller_id, sequence, event_type, run_id, schedule_id,
                        dedupe_key, payload_json, created_at
                    )
                    SELECT
                        NEW.controller_id,
                        COALESCE(MAX(sequence), 0) + 1,
                        CASE
                            WHEN NEW.status = 'queued' AND OLD.status = 'leased'
                                THEN 'run_lease_requeued'
                            WHEN NEW.status = 'queued' THEN 'run_requeued'
                            WHEN NEW.status = 'leased' THEN 'run_leased'
                            WHEN NEW.status = 'running' THEN 'run_started'
                            WHEN NEW.status = 'sleeping' THEN 'run_sleeping'
                            WHEN NEW.status = 'retry_wait' THEN 'run_retry_scheduled'
                            WHEN NEW.status = 'awaiting_input' THEN 'run_awaiting_input'
                            WHEN NEW.status = 'recovery_required' THEN 'recovery_required'
                            WHEN NEW.status = 'succeeded' THEN 'run_completed'
                            WHEN NEW.status = 'failed' THEN 'run_failed'
                            WHEN NEW.status = 'cancelled' THEN 'run_cancelled'
                            WHEN NEW.status = 'dead_letter' THEN 'run_dead_lettered'
                            ELSE 'run_state_changed'
                        END,
                        NEW.id,
                        NEW.schedule_id,
                        'transition:' || NEW.id || ':' || NEW.attempt_count || ':' || NEW.status,
                        json_object(
                            'run_id', NEW.id,
                            'status', NEW.status,
                            'previous_status', OLD.status,
                            'attempt', NEW.attempt_count,
                            'stop_reason', NEW.last_stop_reason,
                            'error', NEW.last_error
                        ),
                        NEW.updated_at
                    FROM mako_controller_events
                    WHERE controller_id = NEW.controller_id;
                END;
                "#,
                )
                .context("Migration 40: atomic Mako run transition journal")?;
            }
            self.set_schema_version_tx(&tx, 40)?;
        }

        // Migration 41: Durable controller-to-runner controls and exact-once
        // scheduled objective delivery. Tool approval decisions must survive
        // daemon restarts and host registration races; the scheduler delivers
        // this outbox only while holding its current process-generation fence.
        if current_version < 41 {
            info!("Running migration 41: Durable Mako control outbox");
            if Self::table_exists(&tx, "mako_controllers")
                && Self::table_exists(&tx, "sessions")
                && Self::table_exists(&tx, "mako_runs")
            {
                tx.execute_batch(
                    r#"
                ALTER TABLE mako_runs
                    ADD COLUMN objective_message_id INTEGER
                    REFERENCES messages(id) ON DELETE SET NULL;

                CREATE TABLE mako_control_outbox (
                    id TEXT PRIMARY KEY,
                    controller_id TEXT NOT NULL,
                    session_id TEXT NOT NULL,
                    run_id TEXT NOT NULL,
                    control_kind TEXT NOT NULL
                        CHECK (control_kind IN ('tool_approval')),
                    dedupe_key TEXT NOT NULL,
                    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
                    status TEXT NOT NULL DEFAULT 'pending'
                        CHECK (status IN ('pending', 'delivered', 'discarded')),
                    attempt_count INTEGER NOT NULL DEFAULT 0
                        CHECK (attempt_count >= 0),
                    available_at TEXT NOT NULL,
                    delivered_at TEXT,
                    last_error TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    UNIQUE (controller_id, control_kind, dedupe_key),
                    FOREIGN KEY (controller_id) REFERENCES mako_controllers(id) ON DELETE CASCADE,
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
                    FOREIGN KEY (run_id) REFERENCES mako_runs(id) ON DELETE CASCADE
                );
                CREATE INDEX idx_mako_control_outbox_pending
                    ON mako_control_outbox(status, available_at, created_at);
                "#,
                )
                .context("Migration 41: durable Mako control outbox")?;
            }
            self.set_schema_version_tx(&tx, 41)?;
        }

        // Migration 42: Repair the atomic Mako transition journal for
        // controllers whose event stream is still empty. The original
        // trigger selected its next sequence from existing event rows, which
        // produced no INSERT candidate at all when there was no prior row.
        // Use a scalar subquery instead so the first status transition is
        // journaled in the same transaction as the run update.
        if current_version < 42 {
            info!("Running migration 42: Complete atomic Mako transition journal");
            if Self::table_exists(&tx, "mako_runs")
                && Self::table_exists(&tx, "mako_controller_events")
            {
                tx.execute_batch(
                    r#"
                DROP TRIGGER IF EXISTS mako_runs_transition_event;
                CREATE TRIGGER mako_runs_transition_event
                AFTER UPDATE OF status ON mako_runs
                WHEN OLD.status <> NEW.status
                BEGIN
                    INSERT OR IGNORE INTO mako_controller_events (
                        controller_id, sequence, event_type, run_id, schedule_id,
                        dedupe_key, payload_json, created_at
                    ) VALUES (
                        NEW.controller_id,
                        (SELECT COALESCE(MAX(sequence), 0) + 1
                         FROM mako_controller_events
                         WHERE controller_id = NEW.controller_id),
                        CASE
                            WHEN NEW.status = 'queued' AND OLD.status = 'leased'
                                THEN 'run_lease_requeued'
                            WHEN NEW.status = 'queued' THEN 'run_requeued'
                            WHEN NEW.status = 'leased' THEN 'run_leased'
                            WHEN NEW.status = 'running' THEN 'run_started'
                            WHEN NEW.status = 'sleeping' THEN 'run_sleeping'
                            WHEN NEW.status = 'retry_wait' THEN 'run_retry_scheduled'
                            WHEN NEW.status = 'awaiting_input' THEN 'run_awaiting_input'
                            WHEN NEW.status = 'recovery_required' THEN 'recovery_required'
                            WHEN NEW.status = 'succeeded' THEN 'run_completed'
                            WHEN NEW.status = 'failed' THEN 'run_failed'
                            WHEN NEW.status = 'cancelled' THEN 'run_cancelled'
                            WHEN NEW.status = 'dead_letter' THEN 'run_dead_lettered'
                            ELSE 'run_state_changed'
                        END,
                        NEW.id,
                        NEW.schedule_id,
                        'transition:' || NEW.id || ':' || NEW.attempt_count || ':' || NEW.status,
                        json_object(
                            'run_id', NEW.id,
                            'status', NEW.status,
                            'previous_status', OLD.status,
                            'attempt', NEW.attempt_count,
                            'stop_reason', NEW.last_stop_reason,
                            'error', NEW.last_error
                        ),
                        NEW.updated_at
                    );
                END;
                "#,
                )
                .context("Migration 42: complete atomic Mako transition journal")?;
            }
            self.set_schema_version_tx(&tx, 42)?;
        }

        // Migration 43: Replace legacy Mako execution payloads with a
        // minimal allow-listed replay form. Earlier builds journaled raw
        // reasoning, provider signatures, tool arguments/results, web
        // bodies, and error copies. Those values are not a durable contract.
        if current_version < 43 {
            info!("Running migration 43: Redact legacy Mako execution journal");
            if Self::table_exists(&tx, "mako_controller_events")
                && Self::column_exists(&tx, "mako_controller_events", "payload_json")
            {
                tx.execute_batch(
                    r#"
                    UPDATE mako_controller_events
                       SET payload_json = CASE
                         WHEN NOT json_valid(payload_json) THEN
                           json_object(
                             'type', 'redacted_invalid_legacy_event',
                             'redacted', json('true')
                           )
                         WHEN event_type = 'agentic_event' THEN
                           json_object(
                             'type', CASE
                               WHEN json_type(payload_json, '$.type') = 'text'
                                 THEN json_extract(payload_json, '$.type')
                               ELSE 'redacted_legacy_event'
                             END,
                             'id', CASE
                               WHEN json_type(payload_json, '$.id') = 'text'
                                 THEN json_extract(payload_json, '$.id')
                             END,
                             'name', CASE
                               WHEN json_type(payload_json, '$.name') = 'text'
                                 THEN json_extract(payload_json, '$.name')
                             END,
                             'tool_call_id', CASE
                               WHEN json_type(payload_json, '$.tool_call_id') = 'text'
                                 THEN json_extract(payload_json, '$.tool_call_id')
                             END,
                             'tool_name', CASE
                               WHEN json_type(payload_json, '$.tool_name') = 'text'
                                 THEN json_extract(payload_json, '$.tool_name')
                             END,
                             'session_id', CASE
                               WHEN json_type(payload_json, '$.session_id') = 'text'
                                 THEN json_extract(payload_json, '$.session_id')
                             END,
                             'status', CASE
                               WHEN json_type(payload_json, '$.status') = 'text'
                                 THEN json_extract(payload_json, '$.status')
                             END,
                             'is_error', CASE
                               WHEN json_type(payload_json, '$.is_error') IN (
                                 'true', 'false', 'integer'
                               ) THEN json_extract(payload_json, '$.is_error')
                             END,
                             'arguments', CASE
                               WHEN json_type(payload_json, '$.arguments') IS NOT NULL THEN
                                 json_object(
                                   'type', json_type(payload_json, '$.arguments'),
                                   'redacted', json('true')
                                 )
                             END,
                             'arguments_redacted', CASE
                               WHEN json_type(payload_json, '$.arguments') IS NOT NULL
                                 THEN json('true')
                               ELSE json('false')
                             END,
                             'redacted', json('true')
                           )
                         ELSE
                           json_object(
                             'run_id', COALESCE(
                               run_id,
                               CASE WHEN json_type(payload_json, '$.run_id') = 'text'
                                 THEN json_extract(payload_json, '$.run_id') END
                             ),
                             'schedule_id', COALESCE(
                               schedule_id,
                               CASE WHEN json_type(payload_json, '$.schedule_id') = 'text'
                                 THEN json_extract(payload_json, '$.schedule_id') END
                             ),
                             'tool_call_id', CASE
                               WHEN json_type(payload_json, '$.tool_call_id') = 'text'
                                 THEN json_extract(payload_json, '$.tool_call_id')
                             END,
                             'pending_id', CASE
                               WHEN json_type(payload_json, '$.pending_id') = 'text'
                                 THEN json_extract(payload_json, '$.pending_id')
                             END,
                             'kind', CASE
                               WHEN json_type(payload_json, '$.kind') = 'text'
                                 THEN json_extract(payload_json, '$.kind')
                             END,
                             'status', CASE
                               WHEN json_type(payload_json, '$.status') = 'text'
                                 THEN json_extract(payload_json, '$.status')
                             END,
                             'previous_status', CASE
                               WHEN json_type(payload_json, '$.previous_status') = 'text'
                                 THEN json_extract(payload_json, '$.previous_status')
                             END,
                             'previous', CASE
                               WHEN json_type(payload_json, '$.previous') = 'text'
                                 THEN json_extract(payload_json, '$.previous')
                             END,
                             'current', CASE
                               WHEN json_type(payload_json, '$.current') = 'text'
                                 THEN json_extract(payload_json, '$.current')
                             END,
                             'attempt', CASE
                               WHEN json_type(payload_json, '$.attempt') = 'integer'
                                 THEN json_extract(payload_json, '$.attempt')
                             END,
                             'attempt_no', CASE
                               WHEN json_type(payload_json, '$.attempt_no') = 'integer'
                                 THEN json_extract(payload_json, '$.attempt_no')
                             END,
                             'revision', CASE
                               WHEN json_type(payload_json, '$.revision') = 'integer'
                                 THEN json_extract(payload_json, '$.revision')
                             END,
                             'approved', CASE
                               WHEN json_type(payload_json, '$.approved') IN (
                                 'true', 'false', 'integer'
                               ) THEN json_extract(payload_json, '$.approved')
                             END,
                             'redacted', json('true')
                           )
                       END;
                    "#,
                )
                .context("Migration 43: redact legacy Mako controller events")?;
            }

            if Self::table_exists(&tx, "mako_runs") {
                if Self::column_exists(&tx, "mako_runs", "last_error") {
                    tx.execute(
                        "UPDATE mako_runs
                            SET last_error = '[redacted legacy execution error]'
                          WHERE last_error IS NOT NULL",
                        [],
                    )?;
                }
                if Self::column_exists(&tx, "mako_runs", "last_stop_reason") {
                    tx.execute(
                        "UPDATE mako_runs
                            SET last_stop_reason = CASE
                              WHEN last_stop_reason IN (
                                'completed', 'failed', 'transient_failure',
                                'invalid_retry_policy', 'retry_schedule_unavailable',
                                'awaiting_input', 'recovery_required',
                                'execution cancelled'
                              ) THEN last_stop_reason
                              ELSE 'redacted_legacy'
                            END
                          WHERE last_stop_reason IS NOT NULL",
                        [],
                    )?;
                }
                if Self::column_exists(&tx, "mako_runs", "outcome_json") {
                    tx.execute_batch(
                        r#"
                        UPDATE mako_runs
                           SET outcome_json = json_object(
                             'kind', CASE
                               WHEN json_valid(outcome_json)
                                AND json_extract(outcome_json, '$.kind') IN (
                                  'succeeded', 'failed', 'retry_scheduled',
                                  'sleeping', 'awaiting_input',
                                  'recovery_required', 'cancelled'
                                ) THEN json_extract(outcome_json, '$.kind')
                               ELSE 'redacted_legacy'
                             END,
                             'redacted', json('true')
                           )
                         WHERE outcome_json IS NOT NULL;
                        "#,
                    )?;
                }
            }
            if Self::table_exists(&tx, "mako_run_attempts") {
                if Self::column_exists(&tx, "mako_run_attempts", "error") {
                    tx.execute(
                        "UPDATE mako_run_attempts
                            SET error = '[redacted legacy execution error]'
                          WHERE error IS NOT NULL",
                        [],
                    )?;
                }
                if Self::column_exists(&tx, "mako_run_attempts", "stop_reason") {
                    tx.execute(
                        "UPDATE mako_run_attempts
                            SET stop_reason = CASE
                              WHEN stop_reason IN (
                                'completed', 'failed', 'transient_failure',
                                'invalid_retry_policy', 'retry_schedule_unavailable',
                                'awaiting_input', 'recovery_required',
                                'execution cancelled'
                              ) THEN stop_reason
                              ELSE 'redacted_legacy'
                            END
                          WHERE stop_reason IS NOT NULL",
                        [],
                    )?;
                }
            }
            if Self::table_exists(&tx, "mako_runtime_state")
                && Self::column_exists(&tx, "mako_runtime_state", "last_error")
            {
                tx.execute(
                    "UPDATE mako_runtime_state
                        SET last_error = '[redacted legacy execution error]'
                      WHERE last_error IS NOT NULL",
                    [],
                )?;
            }
            if Self::table_exists(&tx, "mako_control_outbox") {
                if Self::column_exists(&tx, "mako_control_outbox", "last_error") {
                    tx.execute(
                        "UPDATE mako_control_outbox
                            SET last_error = '[redacted legacy delivery error]'
                          WHERE last_error IS NOT NULL",
                        [],
                    )?;
                }
                if Self::column_exists(&tx, "mako_control_outbox", "payload_json") {
                    tx.execute_batch(
                        r#"
                        UPDATE mako_control_outbox
                           SET payload_json = CASE
                             WHEN json_valid(payload_json) THEN json_object(
                               'tool_call_id', CASE
                                 WHEN json_type(payload_json, '$.tool_call_id') = 'text'
                                   THEN json_extract(payload_json, '$.tool_call_id')
                               END,
                               'approved', CASE
                                 WHEN json_type(payload_json, '$.approved') IN (
                                   'true', 'false', 'integer'
                                 ) THEN json_extract(payload_json, '$.approved')
                               END,
                               'redacted', json('true')
                             )
                             ELSE json_object('redacted', json('true'))
                           END;
                        "#,
                    )?;
                }
            }

            // Keep every future transition event on the same privacy contract
            // as the explicit append boundary. Raw stop reasons and errors
            // remain on the run projection only in their already-redacted
            // scheduler form and never enter replay payloads.
            if Self::table_exists(&tx, "mako_runs")
                && Self::table_exists(&tx, "mako_controller_events")
            {
                tx.execute_batch(
                    r#"
                    DROP TRIGGER IF EXISTS mako_runs_transition_event;
                    CREATE TRIGGER mako_runs_transition_event
                    AFTER UPDATE OF status ON mako_runs
                    WHEN OLD.status <> NEW.status
                    BEGIN
                        INSERT OR IGNORE INTO mako_controller_events (
                            controller_id, sequence, event_type, run_id, schedule_id,
                            dedupe_key, payload_json, created_at
                        ) VALUES (
                            NEW.controller_id,
                            (SELECT COALESCE(MAX(sequence), 0) + 1
                             FROM mako_controller_events
                             WHERE controller_id = NEW.controller_id),
                            CASE
                                WHEN NEW.status = 'queued' AND OLD.status = 'leased'
                                    THEN 'run_lease_requeued'
                                WHEN NEW.status = 'queued' THEN 'run_requeued'
                                WHEN NEW.status = 'leased' THEN 'run_leased'
                                WHEN NEW.status = 'running' THEN 'run_started'
                                WHEN NEW.status = 'sleeping' THEN 'run_sleeping'
                                WHEN NEW.status = 'retry_wait' THEN 'run_retry_scheduled'
                                WHEN NEW.status = 'awaiting_input' THEN 'run_awaiting_input'
                                WHEN NEW.status = 'recovery_required' THEN 'recovery_required'
                                WHEN NEW.status = 'succeeded' THEN 'run_completed'
                                WHEN NEW.status = 'failed' THEN 'run_failed'
                                WHEN NEW.status = 'cancelled' THEN 'run_cancelled'
                                WHEN NEW.status = 'dead_letter' THEN 'run_dead_lettered'
                                ELSE 'run_state_changed'
                            END,
                            NEW.id,
                            NEW.schedule_id,
                            'transition:' || NEW.id || ':' || NEW.attempt_count || ':' || NEW.status,
                            json_object(
                                'run_id', NEW.id,
                                'status', NEW.status,
                                'previous_status', OLD.status,
                                'attempt', NEW.attempt_count,
                                'has_stop_reason', NEW.last_stop_reason IS NOT NULL,
                                'has_error', NEW.last_error IS NOT NULL,
                                'redacted', json('true')
                            ),
                            NEW.updated_at
                        );
                    END;
                    "#,
                )
                .context("Migration 43: install privacy-safe Mako transition journal")?;
            }
            self.set_schema_version_tx(&tx, 43)?;
        }

        tx.commit()?;

        if privacy_cleanup_required {
            // UPDATE redaction alone does not remove prior values from WAL or
            // free pages. Checkpoint, rebuild, and checkpoint again before
            // publishing schema 44 as the durable cleanup-complete marker.
            self.checkpoint_wal_without_busy_readers("pre-VACUUM")?;
            self.conn
                .execute_batch("VACUUM;")
                .context("physically erasing legacy Mako journal payloads")?;
            self.checkpoint_wal_without_busy_readers("post-VACUUM")?;
        }

        // Migration 44: crash-safe physical privacy-cleanup checkpoint. A
        // process that dies after migration 43 commits but before this insert
        // leaves the database at 43; the next opener repeats the idempotent
        // checkpoint/VACUUM and only then advances to 44.
        let finalize_tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)
            .context("acquiring Mako privacy-cleanup checkpoint lock")?;
        finalize_tx
            .execute(
                "INSERT OR IGNORE INTO schema_version (version) VALUES (44)",
                [],
            )
            .context("recording completed Mako privacy cleanup")?;
        finalize_tx.commit()?;

        if privacy_cleanup_requested {
            self.restore_normal_locking_after_privacy_migration()?;
        }

        info!("Migrations complete");
        Ok(())
    }
}

#[cfg(test)]
mod privacy_checkpoint_tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::TempDir;

    #[test]
    fn privacy_checkpoint_rejects_a_pinned_wal_reader() {
        let temp_dir = TempDir::new().expect("temp dir");
        let db_path = temp_dir.path().join("privacy-checkpoint-busy.db");
        let writer = Connection::open(&db_path).expect("open writer");
        writer
            .pragma_update(None, "journal_mode", "WAL")
            .expect("enable WAL");
        writer
            .execute_batch(
                "CREATE TABLE probe (value INTEGER NOT NULL); INSERT INTO probe VALUES (1);",
            )
            .expect("seed probe");
        let database = Database { conn: writer };
        database
            .checkpoint_wal_without_busy_readers("fixture")
            .expect("clear fixture WAL");

        let reader = Connection::open(&db_path).expect("open reader");
        reader.execute_batch("BEGIN").expect("begin read snapshot");
        let count: i64 = reader
            .query_row("SELECT COUNT(*) FROM probe", [], |row| row.get(0))
            .expect("pin read snapshot");
        assert_eq!(count, 1);

        database
            .conn
            .execute("INSERT INTO probe VALUES (2)", [])
            .expect("append newer WAL frame");
        let error = database
            .checkpoint_wal_without_busy_readers("pinned-reader")
            .expect_err("privacy checkpoint must fail while a reader pins the WAL");
        assert!(
            error.to_string().contains("was busy"),
            "unexpected checkpoint error: {error:#}"
        );

        reader.execute_batch("ROLLBACK").expect("release snapshot");
        drop(reader);
        database
            .checkpoint_wal_without_busy_readers("released-reader")
            .expect("checkpoint should succeed after reader release");
    }
}
