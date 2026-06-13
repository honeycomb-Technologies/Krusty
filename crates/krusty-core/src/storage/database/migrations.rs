use anyhow::{Context, Result};
use tracing::{debug, info};

use super::{Database, SCHEMA_VERSION};

impl Database {
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

    /// Run database migrations incrementally
    pub(crate) fn run_migrations(&self) -> Result<()> {
        let current_version = self.get_schema_version();
        debug!(
            "Database schema version: {} (target: {})",
            current_version, SCHEMA_VERSION
        );

        if current_version >= SCHEMA_VERSION {
            return Ok(());
        }

        // Wrap migrations in a transaction for atomicity
        let tx = self.conn.unchecked_transaction()?;

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

        tx.commit()?;

        info!("Migrations complete");
        Ok(())
    }
}
