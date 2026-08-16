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

    /// Rebuild a table by rewriting its CREATE TABLE SQL so CHECK constraints
    /// can accept a renamed discriminator without data loss.
    ///
    /// Optional `value_rewrites` map column values during the copy so data that
    /// only exists under the old CHECK enum (e.g. session_type='mako') can land
    /// in a table whose rewritten CHECK only accepts the canonical value
    /// ('hive'). Plain UPDATE-before-rebuild cannot do this: the old CHECK
    /// rejects the canonical spelling.
    fn rebuild_table_with_sql_rewrite_and_values(
        tx: &rusqlite::Transaction,
        table: &str,
        from: &[&str],
        to: &[&str],
        value_rewrites: &[(&str, &str, &str)],
    ) -> Result<()> {
        assert_eq!(from.len(), to.len());
        let create_sql: String = match tx.query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |row| row.get(0),
        ) {
            Ok(sql) => sql,
            Err(_) => return Ok(()),
        };

        let mut rewritten = create_sql.clone();
        for (old, new) in from.iter().zip(to.iter()) {
            rewritten = rewritten.replace(old, new);
        }
        if rewritten == create_sql && value_rewrites.is_empty() {
            return Ok(());
        }

        let tmp = format!("{table}__hive_rewrite");
        tx.execute_batch(&format!("DROP TABLE IF EXISTS \"{tmp}\";"))?;
        let create_tmp = rewritten.replacen(table, &tmp, 1);
        tx.execute_batch(&create_tmp)
            .with_context(|| format!("create rewritten table {tmp}"))?;

        let has_rows = tx.query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM \"{table}\" LIMIT 1)"),
            [],
            |row| row.get::<_, bool>(0),
        )?;
        // Some migration tests intentionally model only the table being
        // rewritten and omit its referenced parent tables. SQLite resolves
        // those references when an INSERT executes, even if its SELECT would
        // return no rows. Skipping a provably empty copy keeps those synthetic
        // schemas migratable without disabling or weakening production FKs.
        if has_rows {
            let insert_sql = if value_rewrites.is_empty() {
                format!("INSERT INTO \"{tmp}\" SELECT * FROM \"{table}\";")
            } else {
                let mut columns: Vec<String> = Vec::new();
                {
                    let mut stmt = tx
                        .prepare(&format!("PRAGMA table_info(\"{table}\")"))
                        .with_context(|| format!("inspect columns for {table}"))?;
                    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
                    for row in rows {
                        columns.push(row?);
                    }
                }
                ensure!(
                    !columns.is_empty(),
                    "cannot rewrite values for empty table definition {table}"
                );
                let select_list = columns
                    .iter()
                    .map(|column| {
                        if let Some((_, from_value, to_value)) = value_rewrites
                            .iter()
                            .find(|(name, _, _)| *name == column.as_str())
                        {
                            format!(
                                "CASE WHEN \"{column}\" = '{from_value}' THEN '{to_value}' ELSE \"{column}\" END AS \"{column}\""
                            )
                        } else {
                            format!("\"{column}\"")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("INSERT INTO \"{tmp}\" SELECT {select_list} FROM \"{table}\";")
            };
            tx.execute_batch(&insert_sql)
                .with_context(|| format!("copy rows into {tmp}"))?;
        }

        let mut index_sqls: Vec<String> = Vec::new();
        {
            let mut stmt = tx.prepare(
                "SELECT sql FROM sqlite_master
                 WHERE type = 'index'
                   AND tbl_name = ?1
                   AND sql IS NOT NULL",
            )?;
            let rows = stmt.query_map([table], |row| row.get::<_, String>(0))?;
            for row in rows {
                index_sqls.push(row?);
            }
        }

        tx.execute_batch(&format!("DROP TABLE \"{table}\";"))?;
        tx.execute_batch(&format!("ALTER TABLE \"{tmp}\" RENAME TO \"{table}\";"))?;

        for sql in index_sqls {
            tx.execute_batch(&sql)
                .with_context(|| format!("recreate index for {table}: {sql}"))?;
        }
        Ok(())
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
                        CHECK (session_type IN ('chat', 'code', 'hive')),
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
                        CHECK (session_type IN ('chat', 'code', 'hive'));
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
                    model_key_json TEXT,
                    model_catalog_revision TEXT,
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
                        CHECK (namespace IN ('shared', 'hive', 'crew')),
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
                    "ALTER TABLE agent_memories ADD COLUMN namespace TEXT NOT NULL DEFAULT 'shared' CHECK (namespace IN ('shared', 'hive', 'crew'));",
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

        // Migration 45: provider-aware model identity on sessions.
        //
        // Keep the legacy `model` slug for older clients while storing the
        // exact provider/auth/transport key and catalog revision alongside it.
        // Existing rows intentionally remain NULL: guessing a provider from a
        // bare slug would recreate the ambiguity this migration removes.
        if current_version < 45 {
            info!("Running migration 45: provider-aware session model identity");
            let model_tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)
                .context("acquiring provider-aware model migration lock")?;
            if Self::table_exists(&model_tx, "sessions") {
                if !Self::column_exists(&model_tx, "sessions", "model_key_json") {
                    model_tx
                        .execute_batch("ALTER TABLE sessions ADD COLUMN model_key_json TEXT;")
                        .context("Migration 45: add session model key")?;
                }
                if !Self::column_exists(&model_tx, "sessions", "model_catalog_revision") {
                    model_tx
                        .execute_batch(
                            "ALTER TABLE sessions ADD COLUMN model_catalog_revision TEXT;",
                        )
                        .context("Migration 45: add session model catalog revision")?;
                }
            }
            model_tx.execute(
                "INSERT OR IGNORE INTO schema_version (version) VALUES (45)",
                [],
            )?;
            model_tx.commit()?;
        }

        // Migration 46: provider-aware model identity on durable Mako schedules.
        //
        // Scheduled occurrences must retain the same provider/auth/transport
        // selection as the request that created them. Existing bare-model rows
        // remain NULL and use the ambiguity-rejecting legacy execution path.
        if current_version < 46 {
            info!("Running migration 46: provider-aware Mako schedule model identity");
            let mako_model_tx =
                Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)
                    .context("acquiring Mako model identity migration lock")?;
            if Self::table_exists(&mako_model_tx, "mako_schedules") {
                if !Self::column_exists(&mako_model_tx, "mako_schedules", "model_key_json") {
                    mako_model_tx
                        .execute_batch("ALTER TABLE mako_schedules ADD COLUMN model_key_json TEXT;")
                        .context("Migration 46: add Mako schedule model key")?;
                }
                if !Self::column_exists(&mako_model_tx, "mako_schedules", "model_catalog_revision")
                {
                    mako_model_tx
                        .execute_batch(
                            "ALTER TABLE mako_schedules ADD COLUMN model_catalog_revision TEXT;",
                        )
                        .context("Migration 46: add Mako schedule model catalog revision")?;
                }
            }
            mako_model_tx.execute(
                "INSERT OR IGNORE INTO schema_version (version) VALUES (46)",
                [],
            )?;
            mako_model_tx.commit()?;
        }

        // Migration 47: Canonical Goal/Plan workflow state.
        //
        // The legacy `plans` table remains intact as an import/export and
        // rollback surface. Executable workflow state is normalized, revisioned,
        // and append-journaled so reconnects, concurrent clients, and automatic
        // continuation all observe the same durable contract.
        if current_version < 47 {
            info!("Running migration 47: canonical Goal and Plan workflow");
            let workflow_tx =
                Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)
                    .context("acquiring workflow migration lock")?;
            workflow_tx
                .execute_batch(
                    r#"
                    CREATE TABLE IF NOT EXISTS workflow_goals (
                        id TEXT PRIMARY KEY,
                        session_id TEXT NOT NULL,
                        title TEXT NOT NULL,
                        objective TEXT NOT NULL,
                        constraints_json TEXT NOT NULL DEFAULT '[]'
                            CHECK (json_valid(constraints_json)),
                        status TEXT NOT NULL DEFAULT 'draft'
                            CHECK (status IN (
                                'draft', 'active', 'paused', 'blocked',
                                'completed', 'cancelled'
                            )),
                        status_reason TEXT,
                        needs_definition INTEGER NOT NULL DEFAULT 0
                            CHECK (needs_definition IN (0, 1)),
                        revision INTEGER NOT NULL DEFAULT 1
                            CHECK (revision >= 1),
                        token_budget INTEGER CHECK (token_budget IS NULL OR token_budget > 0),
                        tokens_used INTEGER NOT NULL DEFAULT 0
                            CHECK (tokens_used >= 0),
                        source TEXT NOT NULL DEFAULT 'user'
                            CHECK (source IN ('user', 'legacy_import', 'system')),
                        legacy_plan_id TEXT,
                        created_at TEXT NOT NULL,
                        updated_at TEXT NOT NULL,
                        activated_at TEXT,
                        completed_at TEXT,
                        cancelled_at TEXT,
                        FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
                    );
                    CREATE UNIQUE INDEX IF NOT EXISTS idx_workflow_goals_one_unfinished
                        ON workflow_goals(session_id)
                        WHERE status IN ('draft', 'active', 'paused', 'blocked');
                    CREATE UNIQUE INDEX IF NOT EXISTS idx_workflow_goals_legacy_plan
                        ON workflow_goals(legacy_plan_id)
                        WHERE legacy_plan_id IS NOT NULL;
                    CREATE INDEX IF NOT EXISTS idx_workflow_goals_session_updated
                        ON workflow_goals(session_id, updated_at DESC);

                    CREATE TABLE IF NOT EXISTS workflow_goal_criteria (
                        id TEXT PRIMARY KEY,
                        goal_id TEXT NOT NULL,
                        position INTEGER NOT NULL CHECK (position >= 0),
                        description TEXT NOT NULL,
                        required INTEGER NOT NULL DEFAULT 1
                            CHECK (required IN (0, 1)),
                        status TEXT NOT NULL DEFAULT 'pending'
                            CHECK (status IN ('pending', 'passed', 'failed', 'waived')),
                        evidence_json TEXT NOT NULL DEFAULT '[]'
                            CHECK (json_valid(evidence_json)),
                        verifier TEXT,
                        verified_at TEXT,
                        FOREIGN KEY (goal_id) REFERENCES workflow_goals(id) ON DELETE CASCADE,
                        UNIQUE (goal_id, position)
                    );

                    CREATE TABLE IF NOT EXISTS workflow_plan_revisions (
                        id TEXT PRIMARY KEY,
                        goal_id TEXT NOT NULL,
                        revision_number INTEGER NOT NULL CHECK (revision_number >= 1),
                        status TEXT NOT NULL DEFAULT 'proposed'
                            CHECK (status IN (
                                'proposed', 'approved', 'active', 'superseded',
                                'completed', 'cancelled'
                            )),
                        title TEXT NOT NULL,
                        rationale TEXT,
                        source_message_id INTEGER,
                        predecessor_id TEXT,
                        legacy_markdown TEXT,
                        created_at TEXT NOT NULL,
                        approved_at TEXT,
                        completed_at TEXT,
                        FOREIGN KEY (goal_id) REFERENCES workflow_goals(id) ON DELETE CASCADE,
                        FOREIGN KEY (source_message_id) REFERENCES messages(id) ON DELETE SET NULL,
                        FOREIGN KEY (predecessor_id) REFERENCES workflow_plan_revisions(id)
                            ON DELETE SET NULL,
                        UNIQUE (goal_id, revision_number)
                    );
                    CREATE UNIQUE INDEX IF NOT EXISTS idx_workflow_plan_one_active
                        ON workflow_plan_revisions(goal_id)
                        WHERE status = 'active';
                    CREATE INDEX IF NOT EXISTS idx_workflow_plan_goal_revision
                        ON workflow_plan_revisions(goal_id, revision_number DESC);

                    CREATE TABLE IF NOT EXISTS workflow_plan_steps (
                        id TEXT PRIMARY KEY,
                        plan_revision_id TEXT NOT NULL,
                        parent_step_id TEXT,
                        display_key TEXT NOT NULL,
                        position INTEGER NOT NULL CHECK (position >= 0),
                        description TEXT NOT NULL,
                        context TEXT,
                        acceptance_criteria_json TEXT NOT NULL DEFAULT '[]'
                            CHECK (json_valid(acceptance_criteria_json)),
                        required INTEGER NOT NULL DEFAULT 1
                            CHECK (required IN (0, 1)),
                        status TEXT NOT NULL DEFAULT 'pending'
                            CHECK (status IN (
                                'pending', 'in_progress', 'blocked', 'completed',
                                'failed', 'skipped', 'cancelled'
                            )),
                        outcome TEXT,
                        evidence_json TEXT NOT NULL DEFAULT '[]'
                            CHECK (json_valid(evidence_json)),
                        claimed_attempt_id TEXT,
                        revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
                        created_at TEXT NOT NULL,
                        started_at TEXT,
                        completed_at TEXT,
                        FOREIGN KEY (plan_revision_id)
                            REFERENCES workflow_plan_revisions(id) ON DELETE CASCADE,
                        FOREIGN KEY (parent_step_id)
                            REFERENCES workflow_plan_steps(id) ON DELETE CASCADE,
                        UNIQUE (plan_revision_id, display_key)
                    );
                    CREATE UNIQUE INDEX IF NOT EXISTS idx_workflow_one_serial_step
                        ON workflow_plan_steps(plan_revision_id)
                        WHERE status = 'in_progress' AND parent_step_id IS NULL;
                    CREATE INDEX IF NOT EXISTS idx_workflow_steps_plan_position
                        ON workflow_plan_steps(plan_revision_id, position);

                    CREATE TABLE IF NOT EXISTS workflow_step_dependencies (
                        step_id TEXT NOT NULL,
                        depends_on_step_id TEXT NOT NULL,
                        PRIMARY KEY (step_id, depends_on_step_id),
                        CHECK (step_id <> depends_on_step_id),
                        FOREIGN KEY (step_id) REFERENCES workflow_plan_steps(id) ON DELETE CASCADE,
                        FOREIGN KEY (depends_on_step_id)
                            REFERENCES workflow_plan_steps(id) ON DELETE CASCADE
                    );

                    CREATE TABLE IF NOT EXISTS workflow_execution_attempts (
                        id TEXT PRIMARY KEY,
                        goal_id TEXT NOT NULL,
                        plan_revision_id TEXT,
                        step_id TEXT,
                        status TEXT NOT NULL DEFAULT 'running'
                            CHECK (status IN (
                                'running', 'paused', 'succeeded', 'failed', 'cancelled'
                            )),
                        stop_reason TEXT,
                        permission_mode TEXT NOT NULL
                            CHECK (permission_mode IN ('supervised', 'autonomous')),
                        goal_revision_at_start INTEGER NOT NULL,
                        max_turns INTEGER NOT NULL CHECK (max_turns > 0),
                        max_tool_calls INTEGER NOT NULL CHECK (max_tool_calls > 0),
                        max_wall_time_secs INTEGER NOT NULL CHECK (max_wall_time_secs > 0),
                        max_research_actions INTEGER NOT NULL CHECK (max_research_actions > 0),
                        turn_count INTEGER NOT NULL DEFAULT 0 CHECK (turn_count >= 0),
                        tool_call_count INTEGER NOT NULL DEFAULT 0 CHECK (tool_call_count >= 0),
                        research_action_count INTEGER NOT NULL DEFAULT 0
                            CHECK (research_action_count >= 0),
                        progress_revision INTEGER NOT NULL DEFAULT 0
                            CHECK (progress_revision >= 0),
                        blocker_fingerprint TEXT,
                        blocker_streak INTEGER NOT NULL DEFAULT 0
                            CHECK (blocker_streak >= 0),
                        started_at TEXT NOT NULL,
                        updated_at TEXT NOT NULL,
                        ended_at TEXT,
                        FOREIGN KEY (goal_id) REFERENCES workflow_goals(id) ON DELETE CASCADE,
                        FOREIGN KEY (plan_revision_id)
                            REFERENCES workflow_plan_revisions(id) ON DELETE SET NULL,
                        FOREIGN KEY (step_id) REFERENCES workflow_plan_steps(id) ON DELETE SET NULL
                    );
                    CREATE UNIQUE INDEX IF NOT EXISTS idx_workflow_one_running_attempt
                        ON workflow_execution_attempts(goal_id)
                        WHERE status = 'running';
                    CREATE INDEX IF NOT EXISTS idx_workflow_attempt_goal_started
                        ON workflow_execution_attempts(goal_id, started_at DESC);

                    CREATE TABLE IF NOT EXISTS workflow_events (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        session_id TEXT NOT NULL,
                        goal_id TEXT NOT NULL,
                        aggregate_revision INTEGER NOT NULL CHECK (aggregate_revision >= 1),
                        operation_id TEXT NOT NULL UNIQUE,
                        event_type TEXT NOT NULL,
                        actor TEXT NOT NULL,
                        attempt_id TEXT,
                        payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
                        created_at TEXT NOT NULL,
                        FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
                        FOREIGN KEY (goal_id) REFERENCES workflow_goals(id) ON DELETE CASCADE,
                        FOREIGN KEY (attempt_id)
                            REFERENCES workflow_execution_attempts(id) ON DELETE SET NULL,
                        UNIQUE (goal_id, aggregate_revision)
                    );
                    CREATE INDEX IF NOT EXISTS idx_workflow_events_session_revision
                        ON workflow_events(session_id, aggregate_revision);

                    CREATE TABLE IF NOT EXISTS workflow_idempotency (
                        operation_id TEXT PRIMARY KEY,
                        session_id TEXT NOT NULL,
                        action TEXT NOT NULL,
                        result_json TEXT NOT NULL CHECK (json_valid(result_json)),
                        created_at TEXT NOT NULL,
                        FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
                    );
                    "#,
                )
                .context("Migration 47: create canonical workflow schema")?;
            workflow_tx.execute(
                "INSERT OR IGNORE INTO schema_version (version) VALUES (47)",
                [],
            )?;
            workflow_tx.commit()?;
        }

        // Migration 48: canonical mobile notification lifecycle.
        //
        // Device identity and policy are persisted independently from provider
        // acceptance. Notification intents form a durable outbox, Expo tokens
        // cover Android delivery, and ActivityKit tokens are scoped to the
        // session whose Live Activity they update.
        if current_version < 48 {
            info!("Running migration 48: mobile notification lifecycle");
            let notification_tx =
                Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)
                    .context("acquiring notification lifecycle migration lock")?;

            if Self::table_exists(&notification_tx, "apns_devices") {
                if !Self::column_exists(&notification_tx, "apns_devices", "notification_level") {
                    notification_tx.execute_batch(
                        "ALTER TABLE apns_devices
                         ADD COLUMN notification_level TEXT NOT NULL DEFAULT 'important'
                         CHECK (notification_level IN ('all', 'important', 'silent'));",
                    )?;
                }
                if !Self::column_exists(&notification_tx, "apns_devices", "environment") {
                    notification_tx.execute_batch(
                        "ALTER TABLE apns_devices
                         ADD COLUMN environment TEXT NOT NULL DEFAULT 'production'
                         CHECK (environment IN ('sandbox', 'production'));",
                    )?;
                }
                if !Self::column_exists(&notification_tx, "apns_devices", "last_registered_at") {
                    notification_tx.execute_batch(
                        "ALTER TABLE apns_devices ADD COLUMN last_registered_at TEXT;",
                    )?;
                }
                if !Self::column_exists(&notification_tx, "apns_devices", "enabled") {
                    notification_tx.execute_batch(
                        "ALTER TABLE apns_devices
                         ADD COLUMN enabled INTEGER NOT NULL DEFAULT 1
                         CHECK (enabled IN (0, 1));",
                    )?;
                }
            }

            notification_tx
                .execute_batch(
                    r#"
                    CREATE TABLE IF NOT EXISTS expo_push_devices (
                        id TEXT PRIMARY KEY,
                        user_id TEXT,
                        expo_push_token TEXT NOT NULL UNIQUE,
                        platform TEXT NOT NULL,
                        notification_level TEXT NOT NULL DEFAULT 'important'
                            CHECK (notification_level IN ('all', 'important', 'silent')),
                        enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
                        created_at TEXT NOT NULL,
                        last_registered_at TEXT NOT NULL,
                        last_success_at TEXT,
                        last_failure_at TEXT,
                        last_failure_reason TEXT,
                        failure_count INTEGER NOT NULL DEFAULT 0
                    );
                    CREATE INDEX IF NOT EXISTS idx_expo_push_devices_user
                        ON expo_push_devices(user_id);

                    CREATE TABLE IF NOT EXISTS live_activity_tokens (
                        id TEXT PRIMARY KEY,
                        user_id TEXT,
                        session_id TEXT NOT NULL,
                        push_token TEXT NOT NULL UNIQUE,
                        bundle_id TEXT NOT NULL DEFAULT 'io.krusty.mobile',
                        environment TEXT NOT NULL DEFAULT 'production'
                            CHECK (environment IN ('sandbox', 'production')),
                        content_state_json TEXT NOT NULL DEFAULT '{}'
                            CHECK (json_valid(content_state_json)),
                        started_at_ms INTEGER NOT NULL,
                        active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0, 1)),
                        created_at TEXT NOT NULL,
                        updated_at TEXT NOT NULL,
                        ended_at TEXT,
                        FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
                    );
                    CREATE INDEX IF NOT EXISTS idx_live_activity_tokens_session_active
                        ON live_activity_tokens(session_id, active);
                    CREATE INDEX IF NOT EXISTS idx_live_activity_tokens_user
                        ON live_activity_tokens(user_id);

                    CREATE TABLE IF NOT EXISTS notification_intents (
                        id TEXT PRIMARY KEY,
                        operation_id TEXT NOT NULL UNIQUE,
                        user_id TEXT,
                        session_id TEXT,
                        event_type TEXT NOT NULL,
                        payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
                        status TEXT NOT NULL DEFAULT 'pending'
                            CHECK (status IN (
                                'pending', 'dispatching', 'accepted', 'failed',
                                'expired', 'cancelled'
                            )),
                        attempt_count INTEGER NOT NULL DEFAULT 0,
                        available_at TEXT NOT NULL,
                        expires_at TEXT,
                        last_error TEXT,
                        provider_message_id TEXT,
                        created_at TEXT NOT NULL,
                        updated_at TEXT NOT NULL,
                        accepted_at TEXT,
                        FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
                    );
                    CREATE INDEX IF NOT EXISTS idx_notification_intents_pending
                        ON notification_intents(status, available_at);
                    CREATE INDEX IF NOT EXISTS idx_notification_intents_session
                        ON notification_intents(session_id, created_at DESC);
                    CREATE INDEX IF NOT EXISTS idx_notification_intents_user
                        ON notification_intents(user_id, created_at DESC);
                    "#,
                )
                .context("Migration 48: create notification lifecycle schema")?;
            notification_tx.execute(
                "INSERT OR IGNORE INTO schema_version (version) VALUES (48)",
                [],
            )?;
            notification_tx.commit()?;
        }

        // Migration 49: bounded, content-free mobile performance diagnostics.
        // Payloads are operational metadata only; prompts, responses, file
        // contents, terminal output, credentials, and raw URLs are forbidden
        // by the HTTP contract before these rows are written.
        if current_version < 49 {
            info!("Running migration 49: mobile performance diagnostics");
            let diagnostics_tx =
                Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)
                    .context("acquiring mobile diagnostics migration lock")?;
            diagnostics_tx.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS mobile_diagnostic_runs (
                    id TEXT PRIMARY KEY,
                    user_id TEXT,
                    installation_id TEXT NOT NULL,
                    app_version TEXT NOT NULL,
                    build_number TEXT NOT NULL,
                    platform TEXT NOT NULL CHECK (platform IN ('ios', 'android', 'web')),
                    os_version TEXT NOT NULL,
                    device_class TEXT NOT NULL,
                    capture_level TEXT NOT NULL CHECK (capture_level IN ('baseline', 'stress')),
                    started_at_ms INTEGER NOT NULL,
                    ended_at_ms INTEGER,
                    status TEXT NOT NULL DEFAULT 'active'
                        CHECK (status IN ('active', 'completed')),
                    event_count INTEGER NOT NULL DEFAULT 0 CHECK (event_count >= 0),
                    dropped_event_count INTEGER NOT NULL DEFAULT 0 CHECK (dropped_event_count >= 0),
                    byte_count INTEGER NOT NULL DEFAULT 0 CHECK (byte_count >= 0),
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_mobile_diagnostic_runs_user_updated
                    ON mobile_diagnostic_runs(user_id, updated_at DESC);

                CREATE TABLE IF NOT EXISTS mobile_diagnostic_events (
                    run_id TEXT NOT NULL,
                    sequence INTEGER NOT NULL CHECK (sequence >= 0),
                    occurred_at_ms INTEGER NOT NULL,
                    monotonic_ms REAL NOT NULL CHECK (monotonic_ms >= 0),
                    category TEXT NOT NULL,
                    name TEXT NOT NULL,
                    duration_ms REAL CHECK (duration_ms IS NULL OR duration_ms >= 0),
                    severity TEXT NOT NULL DEFAULT 'info'
                        CHECK (severity IN ('debug', 'info', 'warning', 'error', 'fatal')),
                    attributes_json TEXT NOT NULL DEFAULT '{}'
                        CHECK (json_valid(attributes_json)),
                    PRIMARY KEY (run_id, sequence),
                    FOREIGN KEY (run_id) REFERENCES mobile_diagnostic_runs(id) ON DELETE CASCADE
                );
                CREATE INDEX IF NOT EXISTS idx_mobile_diagnostic_events_run_category
                    ON mobile_diagnostic_events(run_id, category, name);

                CREATE TABLE IF NOT EXISTS mobile_diagnostic_native_payloads (
                    run_id TEXT NOT NULL,
                    payload_id TEXT NOT NULL,
                    kind TEXT NOT NULL CHECK (kind IN ('metric', 'diagnostic')),
                    received_at_ms INTEGER NOT NULL,
                    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
                    byte_count INTEGER NOT NULL CHECK (byte_count >= 0),
                    PRIMARY KEY (run_id, payload_id),
                    FOREIGN KEY (run_id) REFERENCES mobile_diagnostic_runs(id) ON DELETE CASCADE
                );
                CREATE INDEX IF NOT EXISTS idx_mobile_diagnostic_native_run_time
                    ON mobile_diagnostic_native_payloads(run_id, received_at_ms ASC);
                "#,
            )?;
            diagnostics_tx.execute(
                "INSERT OR IGNORE INTO schema_version (version) VALUES (49)",
                [],
            )?;
            diagnostics_tx.commit()?;
        }

        // Migration 50: durable steering idempotency receipts.
        //
        // Pending steering rows are deleted when promoted into canonical
        // history, so the messages table alone cannot reject a repeated
        // completion event after promotion. Keep a compact session-scoped
        // receipt for enqueue-once callers; ordinary interactive and process
        // steering retain their existing queue semantics.
        if current_version < 50 {
            info!("Running migration 50: durable steering idempotency receipts");
            let steering_tx =
                Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)
                    .context("acquiring steering idempotency migration lock")?;
            steering_tx.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS steering_idempotency (
                    session_id TEXT NOT NULL,
                    pending_id TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    PRIMARY KEY (session_id, pending_id),
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
                );
                CREATE INDEX IF NOT EXISTS idx_steering_idempotency_created
                    ON steering_idempotency(created_at);
                "#,
            )?;
            steering_tx.execute(
                "INSERT OR IGNORE INTO schema_version (version) VALUES (50)",
                [],
            )?;
            steering_tx.commit()?;
        }

        // Migration 51: persist the parent-chosen identity and exact child
        // capability contract so durable resume does not reconstruct access
        // from a presentation-oriented legacy role.
        if current_version < 51 {
            info!("Running migration 51: delegated child contracts");
            let delegated_tx =
                Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)
                    .context("acquiring delegated child contract migration lock")?;
            if !Self::column_exists(&delegated_tx, "delegated_runs", "child_name") {
                delegated_tx
                    .execute("ALTER TABLE delegated_runs ADD COLUMN child_name TEXT", [])?;
            }
            if !Self::column_exists(&delegated_tx, "delegated_runs", "capabilities_json") {
                delegated_tx.execute(
                    "ALTER TABLE delegated_runs ADD COLUMN capabilities_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(capabilities_json))",
                    [],
                )?;
            }
            delegated_tx.execute(
                "INSERT OR IGNORE INTO schema_version (version) VALUES (51)",
                [],
            )?;
            delegated_tx.commit()?;
        }

        // Migration 52: one durable descendant may claim a delegated run as
        // its continuation origin. A separate claim table lets older databases
        // retain historical duplicate rows while making every new claim
        // atomic across concurrent SQLite connections.
        if current_version < 52 {
            info!("Running migration 52: unique delegated continuation claims");
            let continuation_tx =
                Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)
                    .context("acquiring delegated continuation migration lock")?;
            continuation_tx.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS delegated_run_continuations (
                    resumed_from_run_id TEXT PRIMARY KEY,
                    delegated_run_id TEXT NOT NULL UNIQUE,
                    created_at TEXT NOT NULL,
                    FOREIGN KEY (delegated_run_id) REFERENCES delegated_runs(delegated_run_id)
                        ON DELETE CASCADE
                );

                INSERT OR IGNORE INTO delegated_run_continuations (
                    resumed_from_run_id,
                    delegated_run_id,
                    created_at
                )
                SELECT resumed_from_run_id, delegated_run_id, created_at
                  FROM delegated_runs
                 WHERE resumed_from_run_id IS NOT NULL
                 ORDER BY created_at ASC, delegated_run_id ASC;
                "#,
            )?;
            continuation_tx.execute(
                "INSERT OR IGNORE INTO schema_version (version) VALUES (52)",
                [],
            )?;
            continuation_tx.commit()?;
        }

        // Migration 53: persist background parent-wake intent before a child
        // begins. Terminal artifacts and pending steering are separate durable
        // writes; this flag lets server startup reconcile the crash window
        // between them without mistaking foreground delegated runs for work
        // that promised an autonomous parent continuation.
        if current_version < 53 {
            info!("Running migration 53: delegated background wake intent");
            let wake_tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)
                .context("acquiring delegated wake-intent migration lock")?;
            if !Self::column_exists(&wake_tx, "delegated_runs", "wake_parent") {
                wake_tx.execute(
                    "ALTER TABLE delegated_runs ADD COLUMN wake_parent INTEGER NOT NULL DEFAULT 0 CHECK (wake_parent IN (0, 1))",
                    [],
                )?;
            }
            wake_tx.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_delegated_runs_unqueued_wake
                    ON delegated_runs(wake_parent, stage, completed_at);",
            )?;
            wake_tx.execute(
                "INSERT OR IGNORE INTO schema_version (version) VALUES (53)",
                [],
            )?;
            wake_tx.commit()?;
        }

        // Migration 54: fence server-hosted background Agent execution with a
        // renewable process-owner lease. New launches persist both fields
        // before execution. Existing non-terminal rows remain NULL on purpose:
        // a mixed-version peer may still own them and cannot renew this new
        // contract, so inventing an expiry during migration would be unsafe.
        if current_version < 54 {
            info!("Running migration 54: delegated background host leases");
            let host_lease_tx =
                Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)
                    .context("acquiring delegated host-lease migration lock")?;
            if !Self::column_exists(&host_lease_tx, "delegated_runs", "host_owner_id") {
                host_lease_tx.execute(
                    "ALTER TABLE delegated_runs ADD COLUMN host_owner_id TEXT",
                    [],
                )?;
            }
            if !Self::column_exists(&host_lease_tx, "delegated_runs", "host_lease_expires_at_ms") {
                host_lease_tx.execute(
                    "ALTER TABLE delegated_runs ADD COLUMN host_lease_expires_at_ms INTEGER",
                    [],
                )?;
            }
            host_lease_tx.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_delegated_runs_expired_host_lease
                    ON delegated_runs(wake_parent, stage, host_lease_expires_at_ms);",
            )?;
            host_lease_tx.execute(
                "INSERT OR IGNORE INTO schema_version (version) VALUES (54)",
                [],
            )?;
            host_lease_tx.commit()?;
        }

        // Migration 55: finish Mitsuro/Hive identity in durable schema.
        // - Canonical session_type / memory namespace values are "hive"
        // - Backend tables created as mako_* are renamed to hive_*
        // - CHECK constraints are rewritten to accept the canonical values
        if current_version < 55 {
            info!("Running migration 55: hive table renames and CHECK rewrites");
            // foreign_keys cannot be toggled mid-transaction; disable before
            // opening the rewrite transaction so sessions/agent_memories can
            // be rebuilt without "database table is locked" from child FKs.
            self.conn
                .pragma_update(None, "foreign_keys", "OFF")
                .context("disabling foreign_keys for hive identity rewrite")?;
            let hive_tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)
                .context("acquiring hive identity schema migration lock")?;

            // Prefer renaming physical tables over dual-writing. Indexes that
            // still reference old names are recreated with the table rebuild
            // path below when CHECK SQL must change.
            let renames = [
                ("mako_runtime_state", "hive_runtime_state"),
                ("mako_attention_state", "hive_attention_state"),
                ("mako_profiles", "hive_profiles"),
                ("mako_profile_documents", "hive_profile_documents"),
                ("mako_crew_profiles", "hive_crew_profiles"),
                ("mako_crew_documents", "hive_crew_documents"),
                ("mako_controllers", "hive_controllers"),
                ("mako_schedules", "hive_schedules"),
                ("mako_schedule_occurrences", "hive_schedule_occurrences"),
                ("mako_runs", "hive_runs"),
                ("mako_run_attempts", "hive_run_attempts"),
                ("mako_daemon_leases", "hive_daemon_leases"),
                ("mako_idempotency_keys", "hive_idempotency_keys"),
                ("mako_controller_events", "hive_controller_events"),
                ("mako_learning_runs", "hive_learning_runs"),
                ("mako_learning_candidates", "hive_learning_candidates"),
                ("mako_control_outbox", "hive_control_outbox"),
            ];
            for (from, to) in renames {
                if Self::table_exists(&hive_tx, from) && !Self::table_exists(&hive_tx, to) {
                    hive_tx
                        .execute_batch(&format!("ALTER TABLE \"{from}\" RENAME TO \"{to}\";"))
                        .with_context(|| format!("rename {from} -> {to}"))?;
                }
            }

            // CHECK constraints still only accept the legacy discriminator, so
            // UPDATE-before-rebuild is impossible. Remap values during the
            // table rewrite copy instead.
            Self::rebuild_table_with_sql_rewrite_and_values(
                &hive_tx,
                "sessions",
                &["'mako'"],
                &["'hive'"],
                &[("session_type", "mako", "hive")],
            )
            .context("Migration 55: rebuild sessions CHECK for hive")?;
            if Self::table_exists(&hive_tx, "agent_memories") {
                Self::rebuild_table_with_sql_rewrite_and_values(
                    &hive_tx,
                    "agent_memories",
                    &["'mako'"],
                    &["'hive'"],
                    &[("namespace", "mako", "hive")],
                )
                .context("Migration 55: rebuild agent_memories CHECK for hive")?;
            }

            hive_tx.execute(
                "INSERT OR IGNORE INTO schema_version (version) VALUES (55)",
                [],
            )?;
            hive_tx.commit()?;
            self.conn
                .pragma_update(None, "foreign_keys", "ON")
                .context("re-enabling foreign_keys after hive identity rewrite")?;
        }

        // Migration 56: add the session-level delegation authority. Groups
        // own completion/failure policy, tasks own logical objectives, and an
        // append-preserved attempt ledger owns each execution epoch. The
        // existing delegated_runs row remains the compatibility aggregate.
        if current_version < 56 {
            info!("Running migration 56: delegation groups and tasks");
            let delegation_tx =
                Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)
                    .context("acquiring delegation coordinator migration lock")?;
            delegation_tx.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS delegation_groups (
                    delegation_group_id TEXT PRIMARY KEY,
                    parent_session_id TEXT NOT NULL,
                    parent_tool_call_id TEXT,
                    state TEXT NOT NULL
                        CHECK (state IN (
                            'created', 'queued', 'running', 'ready_for_parent',
                            'synthesizing', 'complete', 'degraded', 'failed', 'cancelled'
                        )),
                    contract_json TEXT NOT NULL CHECK (json_valid(contract_json)),
                    parent_continuation_state TEXT NOT NULL
                        CHECK (parent_continuation_state IN (
                            'not_requested', 'pending', 'queued', 'promoted'
                        )),
                    parent_continuation_id TEXT UNIQUE,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    completed_at TEXT,
                    FOREIGN KEY (parent_session_id) REFERENCES sessions(id) ON DELETE CASCADE
                );

                CREATE TABLE IF NOT EXISTS delegation_tasks (
                    delegation_task_id TEXT PRIMARY KEY,
                    delegation_group_id TEXT NOT NULL,
                    task_key TEXT NOT NULL,
                    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
                    role TEXT NOT NULL
                        CHECK (role IN ('explore', 'build', 'planner', 'verifier')),
                    state TEXT NOT NULL
                        CHECK (state IN (
                            'created', 'queued', 'leased', 'running', 'retrying',
                            'complete', 'degraded', 'failed', 'cancelled'
                        )),
                    specification_json TEXT NOT NULL CHECK (json_valid(specification_json)),
                    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
                    result_json TEXT CHECK (result_json IS NULL OR json_valid(result_json)),
                    error_summary TEXT,
                    lease_owner_id TEXT,
                    lease_expires_at_ms INTEGER,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    completed_at TEXT,
                    FOREIGN KEY (delegation_group_id)
                        REFERENCES delegation_groups(delegation_group_id) ON DELETE CASCADE,
                    UNIQUE (delegation_group_id, task_key),
                    UNIQUE (delegation_group_id, ordinal)
                );

                CREATE TABLE IF NOT EXISTS delegation_events (
                    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
                    parent_session_id TEXT NOT NULL,
                    delegation_group_id TEXT NOT NULL,
                    delegation_task_id TEXT,
                    event_type TEXT NOT NULL CHECK (event_type IN (
                        'group_created', 'group_queued', 'group_state_changed',
                        'task_claimed', 'task_running', 'task_state_changed',
                        'parent_continuation_queued', 'parent_continuation_promoted'
                    )),
                    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
                    created_at TEXT NOT NULL,
                    FOREIGN KEY (parent_session_id) REFERENCES sessions(id) ON DELETE CASCADE,
                    FOREIGN KEY (delegation_group_id)
                        REFERENCES delegation_groups(delegation_group_id) ON DELETE CASCADE,
                    FOREIGN KEY (delegation_task_id)
                        REFERENCES delegation_tasks(delegation_task_id) ON DELETE CASCADE
                );

                CREATE TABLE IF NOT EXISTS delegation_attempts (
                    attempt_id TEXT PRIMARY KEY,
                    delegation_group_id TEXT NOT NULL,
                    delegation_task_id TEXT NOT NULL,
                    attempt_number INTEGER NOT NULL CHECK (attempt_number >= 1),
                    lease_owner_id TEXT NOT NULL,
                    runtime_key TEXT NOT NULL,
                    state TEXT NOT NULL CHECK (state IN (
                        'running', 'complete', 'degraded', 'failed',
                        'cancelled', 'expired'
                    )),
                    artifact_json TEXT CHECK (
                        artifact_json IS NULL OR json_valid(artifact_json)
                    ),
                    error_summary TEXT,
                    started_at TEXT NOT NULL,
                    last_heartbeat_at TEXT NOT NULL,
                    completed_at TEXT,
                    FOREIGN KEY (delegation_group_id)
                        REFERENCES delegation_groups(delegation_group_id) ON DELETE CASCADE,
                    FOREIGN KEY (delegation_task_id)
                        REFERENCES delegation_tasks(delegation_task_id) ON DELETE CASCADE,
                    UNIQUE (delegation_task_id, attempt_number)
                );

                CREATE INDEX IF NOT EXISTS idx_delegation_groups_session_updated
                    ON delegation_groups(parent_session_id, updated_at DESC);
                CREATE INDEX IF NOT EXISTS idx_delegation_groups_parent_tool
                    ON delegation_groups(parent_tool_call_id);
                CREATE INDEX IF NOT EXISTS idx_delegation_groups_state
                    ON delegation_groups(state, updated_at ASC);
                CREATE INDEX IF NOT EXISTS idx_delegation_groups_continuation
                    ON delegation_groups(parent_continuation_state, updated_at ASC);
                CREATE INDEX IF NOT EXISTS idx_delegation_tasks_group_ordinal
                    ON delegation_tasks(delegation_group_id, ordinal ASC);
                CREATE INDEX IF NOT EXISTS idx_delegation_tasks_schedulable
                    ON delegation_tasks(state, lease_expires_at_ms, updated_at ASC);
                CREATE INDEX IF NOT EXISTS idx_delegation_events_session_cursor
                    ON delegation_events(parent_session_id, event_id ASC);
                CREATE INDEX IF NOT EXISTS idx_delegation_events_group_cursor
                    ON delegation_events(delegation_group_id, event_id ASC);
                CREATE INDEX IF NOT EXISTS idx_delegation_attempts_task_number
                    ON delegation_attempts(delegation_task_id, attempt_number ASC);
                CREATE INDEX IF NOT EXISTS idx_delegation_attempts_group_state
                    ON delegation_attempts(delegation_group_id, state, started_at ASC);
                "#,
            )?;
            if !Self::column_exists(&delegation_tx, "delegated_runs", "delegation_group_id") {
                delegation_tx.execute(
                    "ALTER TABLE delegated_runs ADD COLUMN delegation_group_id TEXT",
                    [],
                )?;
            }
            if !Self::column_exists(&delegation_tx, "delegated_runs", "delegation_task_id") {
                delegation_tx.execute(
                    "ALTER TABLE delegated_runs ADD COLUMN delegation_task_id TEXT",
                    [],
                )?;
            }
            if !Self::column_exists(&delegation_tx, "delegated_runs", "attempt_number") {
                delegation_tx.execute(
                    "ALTER TABLE delegated_runs ADD COLUMN attempt_number INTEGER NOT NULL DEFAULT 1 CHECK (attempt_number >= 1)",
                    [],
                )?;
            }
            delegation_tx.execute_batch(
                r#"
                CREATE INDEX IF NOT EXISTS idx_delegated_runs_group_updated
                    ON delegated_runs(delegation_group_id, updated_at DESC);
                CREATE INDEX IF NOT EXISTS idx_delegated_runs_task_attempt
                    ON delegated_runs(delegation_task_id, attempt_number DESC);
                "#,
            )?;
            delegation_tx.execute(
                "INSERT OR IGNORE INTO schema_version (version) VALUES (56)",
                [],
            )?;
            delegation_tx.commit()?;
        }

        // Migration 57: fence aggregate synthesis with the same durable lease
        // discipline as task execution. A process crash in ReadyForParent or
        // Synthesizing can now be reclaimed without allowing two parents to
        // integrate patches or publish the aggregate result concurrently.
        if current_version < 57 {
            info!("Running migration 57: delegation synthesis leases");
            let synthesis_tx =
                Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)
                    .context("acquiring delegation synthesis migration lock")?;
            if !Self::column_exists(&synthesis_tx, "delegation_groups", "synthesis_owner_id") {
                synthesis_tx.execute(
                    "ALTER TABLE delegation_groups ADD COLUMN synthesis_owner_id TEXT",
                    [],
                )?;
            }
            if !Self::column_exists(
                &synthesis_tx,
                "delegation_groups",
                "synthesis_lease_expires_at_ms",
            ) {
                synthesis_tx.execute(
                    "ALTER TABLE delegation_groups ADD COLUMN synthesis_lease_expires_at_ms INTEGER",
                    [],
                )?;
            }
            if !Self::column_exists(
                &synthesis_tx,
                "delegation_groups",
                "synthesis_attempt_count",
            ) {
                synthesis_tx.execute(
                    "ALTER TABLE delegation_groups ADD COLUMN synthesis_attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (synthesis_attempt_count >= 0)",
                    [],
                )?;
            }
            synthesis_tx.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_delegation_groups_synthesis_lease
                    ON delegation_groups(state, synthesis_lease_expires_at_ms);",
            )?;
            synthesis_tx.execute(
                "INSERT OR IGNORE INTO schema_version (version) VALUES (57)",
                [],
            )?;
            synthesis_tx.commit()?;
        }

        // Migration 58: make capacity admission a database authority rather
        // than a process-local assumption. The in-process scheduler remains
        // the fast fairness layer, while these leases provide the hard host,
        // provider-domain, and writer-contention ceiling across every process
        // sharing this database.
        if current_version < 58 {
            info!("Running migration 58: durable delegation capacity authority");
            let capacity_tx =
                Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)
                    .context("acquiring delegation capacity migration lock")?;
            capacity_tx.execute_batch(
                "CREATE TABLE IF NOT EXISTS delegation_capacity_hosts (
                    authority_key TEXT PRIMARY KEY,
                    target_limit INTEGER NOT NULL CHECK (target_limit > 0),
                    minimum_limit INTEGER NOT NULL CHECK (minimum_limit > 0),
                    maximum_limit INTEGER NOT NULL CHECK (maximum_limit >= minimum_limit),
                    ramp_step INTEGER NOT NULL CHECK (ramp_step > 0),
                    healthy_threshold INTEGER NOT NULL CHECK (healthy_threshold > 0),
                    healthy_streak INTEGER NOT NULL DEFAULT 0 CHECK (healthy_streak >= 0),
                    demand_observed INTEGER NOT NULL DEFAULT 0 CHECK (demand_observed IN (0, 1)),
                    default_cooldown_ms INTEGER NOT NULL CHECK (default_cooldown_ms > 0),
                    updated_at_ms INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS delegation_capacity_domains (
                    authority_key TEXT NOT NULL,
                    domain_key TEXT NOT NULL,
                    target_limit INTEGER NOT NULL CHECK (target_limit > 0),
                    healthy_streak INTEGER NOT NULL DEFAULT 0 CHECK (healthy_streak >= 0),
                    demand_observed INTEGER NOT NULL DEFAULT 0 CHECK (demand_observed IN (0, 1)),
                    cooldown_until_ms INTEGER,
                    updated_at_ms INTEGER NOT NULL,
                    PRIMARY KEY (authority_key, domain_key),
                    FOREIGN KEY (authority_key)
                        REFERENCES delegation_capacity_hosts(authority_key) ON DELETE CASCADE
                );

                CREATE TABLE IF NOT EXISTS delegation_capacity_waiters (
                    waiter_sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                    delegation_task_id TEXT NOT NULL UNIQUE,
                    lease_owner_id TEXT NOT NULL,
                    authority_key TEXT NOT NULL,
                    domain_key TEXT NOT NULL,
                    partition_key TEXT NOT NULL,
                    scheduling_class TEXT NOT NULL CHECK (scheduling_class IN (
                        'read_only', 'write_shared', 'write_isolated', 'verification'
                    )),
                    isolation_group TEXT,
                    lease_expires_at_ms INTEGER NOT NULL,
                    enqueued_at_ms INTEGER NOT NULL,
                    FOREIGN KEY (delegation_task_id)
                        REFERENCES delegation_tasks(delegation_task_id) ON DELETE CASCADE,
                    FOREIGN KEY (authority_key)
                        REFERENCES delegation_capacity_hosts(authority_key) ON DELETE CASCADE
                );

                CREATE TABLE IF NOT EXISTS delegation_capacity_leases (
                    delegation_task_id TEXT PRIMARY KEY,
                    lease_owner_id TEXT NOT NULL,
                    authority_key TEXT NOT NULL,
                    domain_key TEXT NOT NULL,
                    partition_key TEXT NOT NULL,
                    scheduling_class TEXT NOT NULL CHECK (scheduling_class IN (
                        'read_only', 'write_shared', 'write_isolated', 'verification'
                    )),
                    isolation_group TEXT,
                    waiter_sequence INTEGER NOT NULL,
                    lease_expires_at_ms INTEGER NOT NULL,
                    admitted_at_ms INTEGER NOT NULL,
                    FOREIGN KEY (delegation_task_id)
                        REFERENCES delegation_tasks(delegation_task_id) ON DELETE CASCADE,
                    FOREIGN KEY (authority_key)
                        REFERENCES delegation_capacity_hosts(authority_key) ON DELETE CASCADE
                );

                CREATE INDEX IF NOT EXISTS idx_delegation_capacity_waiters_order
                    ON delegation_capacity_waiters(authority_key, domain_key, waiter_sequence);
                CREATE INDEX IF NOT EXISTS idx_delegation_capacity_waiters_expiry
                    ON delegation_capacity_waiters(lease_expires_at_ms);
                CREATE INDEX IF NOT EXISTS idx_delegation_capacity_leases_host
                    ON delegation_capacity_leases(authority_key, lease_expires_at_ms);
                CREATE INDEX IF NOT EXISTS idx_delegation_capacity_leases_domain
                    ON delegation_capacity_leases(authority_key, domain_key, lease_expires_at_ms);
                CREATE INDEX IF NOT EXISTS idx_delegation_capacity_leases_writer
                    ON delegation_capacity_leases(authority_key, partition_key, scheduling_class);
                ",
            )?;
            capacity_tx.execute(
                "INSERT OR IGNORE INTO schema_version (version) VALUES (58)",
                [],
            )?;
            capacity_tx.commit()?;
        }

        // Migration 59: persist an immutable, versioned executor envelope for
        // detached Chat/Code tasks. The envelope contains only reconstruction
        // metadata and a digest of the already-bounded task objective; it does
        // not duplicate parent transcripts, raw prompts, or tool outputs.
        if current_version < 59 {
            info!("Running migration 59: detached delegation executor envelopes");
            let envelope_tx =
                Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)
                    .context("acquiring delegation executor envelope migration lock")?;
            if !Self::column_exists(
                &envelope_tx,
                "delegation_tasks",
                "executor_envelope_version",
            ) {
                envelope_tx.execute(
                    "ALTER TABLE delegation_tasks ADD COLUMN executor_envelope_version INTEGER",
                    [],
                )?;
            }
            if !Self::column_exists(&envelope_tx, "delegation_tasks", "executor_envelope_json") {
                envelope_tx.execute(
                    "ALTER TABLE delegation_tasks ADD COLUMN executor_envelope_json TEXT",
                    [],
                )?;
            }
            envelope_tx.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_delegation_tasks_replayable
                    ON delegation_tasks(executor_envelope_version, state, delegation_group_id);",
            )?;
            envelope_tx.execute(
                "INSERT OR IGNORE INTO schema_version (version) VALUES (59)",
                [],
            )?;
            envelope_tx.commit()?;
        }

        // Migration 60: elect exactly one recovery host for a replayable
        // detached group across startup and periodic reconciliation scans.
        if current_version < 60 {
            info!("Running migration 60: delegation replay owner leases");
            let replay_tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)
                .context("acquiring delegation replay lease migration lock")?;
            if !Self::column_exists(&replay_tx, "delegation_groups", "replay_owner_id") {
                replay_tx.execute(
                    "ALTER TABLE delegation_groups ADD COLUMN replay_owner_id TEXT",
                    [],
                )?;
            }
            if !Self::column_exists(
                &replay_tx,
                "delegation_groups",
                "replay_lease_expires_at_ms",
            ) {
                replay_tx.execute(
                    "ALTER TABLE delegation_groups ADD COLUMN replay_lease_expires_at_ms INTEGER",
                    [],
                )?;
            }
            if !Self::column_exists(&replay_tx, "delegation_groups", "replay_attempt_count") {
                replay_tx.execute(
                    "ALTER TABLE delegation_groups ADD COLUMN replay_attempt_count INTEGER NOT NULL DEFAULT 0",
                    [],
                )?;
            }
            replay_tx.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_delegation_groups_replay_lease
                    ON delegation_groups(state, replay_lease_expires_at_ms, delegation_group_id);",
            )?;
            replay_tx.execute(
                "INSERT OR IGNORE INTO schema_version (version) VALUES (60)",
                [],
            )?;
            replay_tx.commit()?;
        }

        // Migration 61: event kinds are an append-only protocol surface, not a
        // closed lifecycle state machine. Keep group/task state CHECKs closed,
        // but allow newer servers to persist event kinds that older clients
        // safely project as `Other`/an opaque string.
        if current_version < 61 {
            info!("Running migration 61: extensible delegation event kinds");
            let event_tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)
                .context("acquiring delegation event compatibility migration lock")?;
            if Self::table_exists(&event_tx, "delegation_events") {
                Self::rebuild_table_with_sql_rewrite_and_values(
                    &event_tx,
                    "delegation_events",
                    &[r#"event_type TEXT NOT NULL CHECK (event_type IN (
                        'group_created', 'group_queued', 'group_state_changed',
                        'task_claimed', 'task_running', 'task_state_changed',
                        'parent_continuation_queued', 'parent_continuation_promoted'
                    ))"#],
                    &["event_type TEXT NOT NULL"],
                    &[],
                )
                .context("Migration 61: remove delegation event kind CHECK")?;

                let create_sql: String = event_tx.query_row(
                    "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'delegation_events'",
                    [],
                    |row| row.get(0),
                )?;
                ensure!(
                    !create_sql.contains("event_type IN"),
                    "Migration 61 could not remove the delegation event kind CHECK"
                );
            }
            event_tx.execute(
                "INSERT OR IGNORE INTO schema_version (version) VALUES (61)",
                [],
            )?;
            event_tx.commit()?;
        }

        // Migration 62: durable conversation-list organization. Pinning is an
        // ordering preference, while archiving is a reversible visibility
        // state; neither mutates conversation history or project files.
        if current_version < 62 {
            info!("Running migration 62: session pin and archive metadata");
            let session_list_tx =
                Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)
                    .context("acquiring session list metadata migration lock")?;
            if Self::table_exists(&session_list_tx, "sessions") {
                if !Self::column_exists(&session_list_tx, "sessions", "pinned_at") {
                    session_list_tx
                        .execute_batch("ALTER TABLE sessions ADD COLUMN pinned_at TEXT;")?;
                }
                if !Self::column_exists(&session_list_tx, "sessions", "archived_at") {
                    session_list_tx
                        .execute_batch("ALTER TABLE sessions ADD COLUMN archived_at TEXT;")?;
                }
                if Self::column_exists(&session_list_tx, "sessions", "updated_at") {
                    session_list_tx.execute_batch(
                        "CREATE INDEX IF NOT EXISTS idx_sessions_archive_pin_updated
                            ON sessions(archived_at, pinned_at, updated_at DESC);",
                    )?;
                }
            }
            session_list_tx.execute(
                "INSERT OR IGNORE INTO schema_version (version) VALUES (62)",
                [],
            )?;
            session_list_tx.commit()?;
        }

        // Migration 63: first-class Hive Worker identities.
        //
        // A Worker is a durable product identity (persona documents, frozen
        // provider/model choice, autonomy policy, private DM session) layered
        // over the existing controller/run machinery. Existing crew profiles
        // and the durable Hive companion session are backfilled as Workers,
        // and the daemon lease claimant on hive_run_attempts is renamed to
        // executor_id so "worker" refers only to the product concept.
        if current_version < 63 {
            info!("Running migration 63: Hive Worker identities");
            let worker_tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)
                .context("acquiring Hive worker identity migration lock")?;

            worker_tx
                .execute_batch(
                    r#"
                    CREATE TABLE IF NOT EXISTS hive_workers (
                        id TEXT PRIMARY KEY,
                        user_id TEXT,
                        slug TEXT NOT NULL,
                        display_name TEXT NOT NULL,
                        avatar_color TEXT,
                        model TEXT,
                        model_key_json TEXT
                            CHECK (model_key_json IS NULL OR json_valid(model_key_json)),
                        model_catalog_revision TEXT,
                        permission_mode TEXT NOT NULL DEFAULT 'autonomous'
                            CHECK (permission_mode IN ('supervised', 'autonomous')),
                        autonomy TEXT NOT NULL DEFAULT 'manual'
                            CHECK (autonomy IN ('manual', 'scheduled', 'always_on')),
                        heartbeat_interval_secs INTEGER
                            CHECK (
                                heartbeat_interval_secs IS NULL
                                OR heartbeat_interval_secs > 0
                            ),
                        status TEXT NOT NULL DEFAULT 'active'
                            CHECK (status IN ('active', 'paused', 'archived')),
                        dm_session_id TEXT UNIQUE
                            REFERENCES sessions(id) ON DELETE SET NULL,
                        memory_namespace_id TEXT NOT NULL,
                        created_at TEXT NOT NULL,
                        updated_at TEXT NOT NULL
                    );
                    CREATE UNIQUE INDEX IF NOT EXISTS idx_hive_workers_active_slug
                        ON hive_workers(COALESCE(user_id, ''), slug)
                        WHERE status <> 'archived';
                    CREATE INDEX IF NOT EXISTS idx_hive_workers_owner_status
                        ON hive_workers(user_id, status);

                    CREATE TABLE IF NOT EXISTS hive_worker_documents (
                        worker_id TEXT NOT NULL,
                        kind TEXT NOT NULL CHECK (kind IN ('identity', 'soul')),
                        content TEXT NOT NULL,
                        updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                        PRIMARY KEY (worker_id, kind),
                        FOREIGN KEY (worker_id)
                            REFERENCES hive_workers(id) ON DELETE CASCADE
                    );
                    "#,
                )
                .context("Migration 63: create Hive worker tables")?;

            // Nullable linkage columns. Existing rows keep crew_slug as the
            // transitional identity; new call sites write worker_id.
            for table in [
                "hive_controllers",
                "hive_runs",
                "hive_runtime_state",
                "hive_schedules",
            ] {
                if Self::table_exists(&worker_tx, table)
                    && !Self::column_exists(&worker_tx, table, "worker_id")
                {
                    worker_tx
                        .execute_batch(&format!(
                            "ALTER TABLE {table} ADD COLUMN worker_id TEXT
                             REFERENCES hive_workers(id) ON DELETE SET NULL;"
                        ))
                        .with_context(|| format!("Migration 63: add {table}.worker_id"))?;
                }
                if Self::table_exists(&worker_tx, table) {
                    worker_tx.execute_batch(&format!(
                        "CREATE INDEX IF NOT EXISTS idx_{table}_worker
                            ON {table}(worker_id) WHERE worker_id IS NOT NULL;"
                    ))?;
                }
            }

            // Free "worker" for the product concept: the run-attempt claimant
            // is the daemon executor instance, not a Hive Worker.
            if Self::table_exists(&worker_tx, "hive_run_attempts")
                && Self::column_exists(&worker_tx, "hive_run_attempts", "worker_id")
                && !Self::column_exists(&worker_tx, "hive_run_attempts", "executor_id")
            {
                worker_tx
                    .execute_batch(
                        "ALTER TABLE hive_run_attempts RENAME COLUMN worker_id TO executor_id;",
                    )
                    .context("Migration 63: rename attempt claimant to executor_id")?;
            }

            let now = chrono::Utc::now().to_rfc3339();

            // Backfill: every crew profile becomes a Worker owned by the
            // profile's user (NULL user_id = local), keeping the crew slug as
            // the memory namespace so existing crew memories stay reachable.
            if Self::table_exists(&worker_tx, "hive_crew_profiles")
                && Self::table_exists(&worker_tx, "hive_profiles")
            {
                let mut crew_rows: Vec<(Option<String>, String)> = Vec::new();
                {
                    let mut statement = worker_tx.prepare(
                        "SELECT p.user_id, cp.slug
                         FROM hive_crew_profiles cp
                         JOIN hive_profiles p ON p.id = cp.profile_id
                         ORDER BY cp.slug",
                    )?;
                    let rows = statement.query_map([], |row| {
                        Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?))
                    })?;
                    for row in rows {
                        crew_rows.push(row?);
                    }
                }
                for (user_id, slug) in &crew_rows {
                    worker_tx.execute(
                        "INSERT OR IGNORE INTO hive_workers (
                             id, user_id, slug, display_name, permission_mode,
                             autonomy, status, memory_namespace_id,
                             created_at, updated_at
                         ) VALUES (
                             ?1, ?2, ?3, ?4, 'autonomous',
                             'manual', 'active', ?3, ?5, ?5
                         )",
                        rusqlite::params![
                            uuid::Uuid::new_v4().to_string(),
                            user_id,
                            slug,
                            crate::storage::hive_workers::display_name_from_slug(slug),
                            now,
                        ],
                    )?;
                }
                if Self::table_exists(&worker_tx, "hive_crew_documents") {
                    worker_tx
                        .execute(
                            "INSERT OR IGNORE INTO hive_worker_documents
                                 (worker_id, kind, content, updated_at)
                             SELECT w.id, cd.kind, cd.content, cd.updated_at
                             FROM hive_crew_documents cd
                             JOIN hive_profiles p ON p.id = cd.profile_id
                             JOIN hive_workers w
                               ON w.slug = cd.slug
                              AND COALESCE(w.user_id, '') = COALESCE(p.user_id, '')
                              AND w.status <> 'archived'
                             WHERE cd.kind IN ('identity', 'soul')",
                            [],
                        )
                        .context("Migration 63: copy crew documents to workers")?;
                }
            }

            // Backfill: the durable Hive companion chat becomes the default
            // "assistant" Worker with the companion as its DM session.
            let sessions_ready = Self::table_exists(&worker_tx, "sessions")
                && [
                    "title",
                    "session_type",
                    "parent_session_id",
                    "project_dir",
                    "user_id",
                    "updated_at",
                ]
                .iter()
                .all(|column| Self::column_exists(&worker_tx, "sessions", column));
            if sessions_ready {
                let mut companions: Vec<(String, Option<String>)> = Vec::new();
                {
                    let mut statement = worker_tx.prepare(
                        "SELECT id, user_id FROM sessions
                         WHERE title = 'Hive' AND session_type = 'hive'
                           AND parent_session_id IS NULL AND project_dir IS NULL
                         ORDER BY updated_at ASC",
                    )?;
                    let rows = statement.query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
                    })?;
                    for row in rows {
                        companions.push(row?);
                    }
                }
                let mut seen_owners = std::collections::HashSet::new();
                for (session_id, user_id) in &companions {
                    // Mirror ensure_hive_main_session: one companion per
                    // owner, preferring the oldest matching session.
                    if !seen_owners.insert(user_id.clone().unwrap_or_default()) {
                        continue;
                    }
                    worker_tx.execute(
                        "INSERT OR IGNORE INTO hive_workers (
                             id, user_id, slug, display_name, permission_mode,
                             autonomy, status, dm_session_id,
                             memory_namespace_id, created_at, updated_at
                         ) VALUES (
                             ?1, ?2, 'assistant', 'Assistant', 'autonomous',
                             'manual', 'active', ?3,
                             'assistant', ?4, ?4
                         )",
                        rusqlite::params![
                            uuid::Uuid::new_v4().to_string(),
                            user_id,
                            session_id,
                            now,
                        ],
                    )?;
                    // If an "assistant" Worker already existed for this owner
                    // (e.g. a crew member with that slug), bind the companion
                    // as its DM session instead of leaving it dangling.
                    worker_tx.execute(
                        "UPDATE hive_workers
                         SET dm_session_id = ?1, updated_at = ?2
                         WHERE slug = 'assistant'
                           AND COALESCE(user_id, '') = COALESCE(?3, '')
                           AND status <> 'archived'
                           AND dm_session_id IS NULL
                           AND NOT EXISTS (
                               SELECT 1 FROM hive_workers bound
                               WHERE bound.dm_session_id = ?1
                           )",
                        rusqlite::params![session_id, now, user_id],
                    )?;
                }
            }

            // Link controllers whose session is a Worker's DM session (the
            // Worker's serialized execution lane).
            if Self::table_exists(&worker_tx, "hive_controllers")
                && Self::column_exists(&worker_tx, "hive_controllers", "worker_id")
            {
                worker_tx.execute(
                    "UPDATE hive_controllers
                     SET worker_id = (
                         SELECT w.id FROM hive_workers w
                         WHERE w.dm_session_id = hive_controllers.session_id
                           AND w.status <> 'archived'
                     )
                     WHERE worker_id IS NULL
                       AND EXISTS (
                           SELECT 1 FROM hive_workers w
                           WHERE w.dm_session_id = hive_controllers.session_id
                             AND w.status <> 'archived'
                       )",
                    [],
                )?;
            }

            // Dual-read transition: map persisted crew assignments onto the
            // backfilled Workers where the slugs match within one owner.
            if Self::table_exists(&worker_tx, "hive_runtime_state")
                && Self::column_exists(&worker_tx, "hive_runtime_state", "crew_slug")
                && Self::column_exists(&worker_tx, "hive_runtime_state", "worker_id")
                && Self::table_exists(&worker_tx, "sessions")
                && Self::column_exists(&worker_tx, "sessions", "user_id")
            {
                worker_tx.execute(
                    "UPDATE hive_runtime_state
                     SET worker_id = (
                         SELECT w.id FROM hive_workers w
                         JOIN sessions s ON s.id = hive_runtime_state.session_id
                         WHERE w.slug = hive_runtime_state.crew_slug
                           AND COALESCE(w.user_id, '') = COALESCE(s.user_id, '')
                           AND w.status <> 'archived'
                     )
                     WHERE crew_slug IS NOT NULL
                       AND worker_id IS NULL
                       AND EXISTS (
                           SELECT 1 FROM hive_workers w
                           JOIN sessions s ON s.id = hive_runtime_state.session_id
                           WHERE w.slug = hive_runtime_state.crew_slug
                             AND COALESCE(w.user_id, '') = COALESCE(s.user_id, '')
                             AND w.status <> 'archived'
                       )",
                    [],
                )?;
            }
            if Self::table_exists(&worker_tx, "hive_schedules")
                && Self::column_exists(&worker_tx, "hive_schedules", "crew_slug")
                && Self::column_exists(&worker_tx, "hive_schedules", "worker_id")
                && Self::table_exists(&worker_tx, "hive_controllers")
            {
                worker_tx.execute(
                    "UPDATE hive_schedules
                     SET worker_id = (
                         SELECT w.id FROM hive_workers w
                         JOIN hive_controllers c ON c.id = hive_schedules.controller_id
                         WHERE w.slug = hive_schedules.crew_slug
                           AND COALESCE(w.user_id, '') = COALESCE(c.user_id, '')
                           AND w.status <> 'archived'
                     )
                     WHERE crew_slug IS NOT NULL
                       AND worker_id IS NULL
                       AND EXISTS (
                           SELECT 1 FROM hive_workers w
                           JOIN hive_controllers c ON c.id = hive_schedules.controller_id
                           WHERE w.slug = hive_schedules.crew_slug
                             AND COALESCE(w.user_id, '') = COALESCE(c.user_id, '')
                             AND w.status <> 'archived'
                       )",
                    [],
                )?;
            }

            worker_tx.execute(
                "INSERT OR IGNORE INTO schema_version (version) VALUES (63)",
                [],
            )?;
            worker_tx.commit()?;
        }

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

#[cfg(test)]
mod delegation_event_migration_tests {
    use super::*;
    use rusqlite::{params, Connection};
    use tempfile::TempDir;

    #[test]
    fn migration_61_preserves_preview_events_and_accepts_future_kinds() {
        let temp_dir = TempDir::new().expect("temp dir");
        let db_path = temp_dir.path().join("delegation-event-compatibility.db");
        let fixture = Connection::open(&db_path).expect("open preview fixture");
        fixture
            .execute_batch(
                r#"
                CREATE TABLE schema_version (
                    version INTEGER PRIMARY KEY,
                    applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                );
                INSERT INTO schema_version (version) VALUES (60);

                CREATE TABLE sessions (id TEXT PRIMARY KEY);
                CREATE TABLE delegation_groups (delegation_group_id TEXT PRIMARY KEY);
                CREATE TABLE delegation_tasks (delegation_task_id TEXT PRIMARY KEY);
                INSERT INTO sessions (id) VALUES ('session-1');
                INSERT INTO delegation_groups (delegation_group_id) VALUES ('group-1');
                INSERT INTO delegation_tasks (delegation_task_id) VALUES ('task-1');

                CREATE TABLE delegation_events (
                    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
                    parent_session_id TEXT NOT NULL,
                    delegation_group_id TEXT NOT NULL,
                    delegation_task_id TEXT,
                    event_type TEXT NOT NULL CHECK (event_type IN (
                        'group_created', 'group_queued', 'group_state_changed',
                        'task_claimed', 'task_running', 'task_state_changed',
                        'parent_continuation_queued', 'parent_continuation_promoted'
                    )),
                    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
                    created_at TEXT NOT NULL,
                    FOREIGN KEY (parent_session_id) REFERENCES sessions(id) ON DELETE CASCADE,
                    FOREIGN KEY (delegation_group_id)
                        REFERENCES delegation_groups(delegation_group_id) ON DELETE CASCADE,
                    FOREIGN KEY (delegation_task_id)
                        REFERENCES delegation_tasks(delegation_task_id) ON DELETE CASCADE
                );
                CREATE INDEX idx_delegation_events_session_cursor
                    ON delegation_events(parent_session_id, event_id ASC);
                CREATE INDEX idx_delegation_events_group_cursor
                    ON delegation_events(delegation_group_id, event_id ASC);
                INSERT INTO delegation_events (
                    parent_session_id, delegation_group_id, delegation_task_id,
                    event_type, payload_json, created_at
                ) VALUES (
                    'session-1', 'group-1', 'task-1', 'task_running', '{}',
                    '2026-08-08T00:00:00Z'
                );
                "#,
            )
            .expect("seed schema-60 preview database");
        drop(fixture);

        let database = Database::new(&db_path).expect("migrate preview database");
        assert_eq!(database.get_schema_version(), 63);
        database
            .conn()
            .execute(
                "INSERT INTO delegation_events (
                    parent_session_id, delegation_group_id, delegation_task_id,
                    event_type, payload_json, created_at
                 ) VALUES (?1, ?2, NULL, ?3, ?4, ?5)",
                params![
                    "session-1",
                    "group-1",
                    "future_scheduler_event",
                    r#"{"domain":"workspace"}"#,
                    "2026-08-08T00:00:01Z"
                ],
            )
            .expect("persist a future event kind");

        let event_kinds = database
            .conn()
            .prepare("SELECT event_type FROM delegation_events ORDER BY event_id ASC")
            .expect("prepare event query")
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query event kinds")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("collect event kinds");
        assert_eq!(event_kinds, ["task_running", "future_scheduler_event"]);

        let index_count: i64 = database
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'index'
                   AND name IN (
                       'idx_delegation_events_session_cursor',
                       'idx_delegation_events_group_cursor'
                   )",
                [],
                |row| row.get(0),
            )
            .expect("count restored event indexes");
        assert_eq!(index_count, 2);
    }

    #[test]
    fn migration_61_handles_an_empty_synthetic_event_table_without_parent_tables() {
        let temp_dir = TempDir::new().expect("temp dir");
        let db_path = temp_dir.path().join("synthetic-delegation-events.db");
        let fixture = Connection::open(&db_path).expect("open synthetic fixture");
        fixture
            .execute_batch(
                r#"
                CREATE TABLE schema_version (
                    version INTEGER PRIMARY KEY,
                    applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                );
                INSERT INTO schema_version (version) VALUES (60);
                CREATE TABLE delegation_events (
                    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
                    parent_session_id TEXT NOT NULL,
                    delegation_group_id TEXT NOT NULL,
                    delegation_task_id TEXT,
                    event_type TEXT NOT NULL CHECK (event_type IN (
                        'group_created', 'group_queued', 'group_state_changed',
                        'task_claimed', 'task_running', 'task_state_changed',
                        'parent_continuation_queued', 'parent_continuation_promoted'
                    )),
                    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
                    created_at TEXT NOT NULL,
                    FOREIGN KEY (parent_session_id) REFERENCES sessions(id) ON DELETE CASCADE,
                    FOREIGN KEY (delegation_group_id)
                        REFERENCES delegation_groups(delegation_group_id) ON DELETE CASCADE,
                    FOREIGN KEY (delegation_task_id)
                        REFERENCES delegation_tasks(delegation_task_id) ON DELETE CASCADE
                );
                "#,
            )
            .expect("seed synthetic schema-60 database");
        drop(fixture);

        let database = Database::new(&db_path).expect("migrate synthetic database");
        assert_eq!(database.get_schema_version(), 63);
        let create_sql: String = database
            .conn()
            .query_row(
                "SELECT sql FROM sqlite_master
                 WHERE type = 'table' AND name = 'delegation_events'",
                [],
                |row| row.get(0),
            )
            .expect("read migrated event schema");
        assert!(!create_sql.contains("event_type IN"));
        let foreign_key_count: i64 = database
            .conn()
            .prepare("PRAGMA foreign_key_list(delegation_events)")
            .expect("prepare event foreign keys")
            .query_map([], |_| Ok(()))
            .expect("query event foreign keys")
            .count() as i64;
        assert_eq!(
            foreign_key_count, 3,
            "migration must preserve production FKs"
        );
    }
}
