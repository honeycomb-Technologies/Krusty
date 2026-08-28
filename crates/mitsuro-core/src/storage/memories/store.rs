use anyhow::{bail, Context, Result};
use chrono::{Duration, Utc};
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use uuid::Uuid;

use crate::storage::database::Database;

use super::model::{
    AgentMemory, AgentMemoryRevision, CanonicalMemoryInput, MemoryAclScope, MemoryNamespace,
    MemoryRevisionEvent, MemorySensitivity, MemoryType,
};
use super::query::{build_list_query, row_to_memory, MEMORY_SELECT_COLUMNS};

/// Reader identity for Hive memory injection. Worker-private rows are
/// visible only when `worker_namespace_id` matches; group/conversation
/// rows require an exact conversation id.
#[derive(Debug, Clone, Default)]
pub struct HiveMemoryReader<'a> {
    pub user_id: Option<&'a str>,
    pub project_dir: Option<&'a str>,
    pub worker_namespace_id: Option<&'a str>,
    pub conversation_id: Option<&'a str>,
    pub group_id: Option<&'a str>,
}

pub struct MemoryStore {
    db: Database,
}

impl MemoryStore {
    /// Open a memory store.
    ///
    /// Production schema upgrades are migration-owned. The `CREATE IF NOT
    /// EXISTS` statements retain the historical standalone-store behavior for
    /// a database where these tables have never existed; they deliberately do
    /// not attempt to alter a legacy table in place.
    pub fn new(db: Database) -> Self {
        let _ = db.conn().execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS agent_memories (
                id TEXT PRIMARY KEY,
                memory_type TEXT NOT NULL CHECK(memory_type IN ('user', 'feedback', 'project', 'reference')),
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                project_dir TEXT,
                user_id TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                canonical_key TEXT,
                namespace TEXT NOT NULL DEFAULT 'shared' CHECK(namespace IN ('shared', 'hive', 'crew')),
                namespace_id TEXT,
                status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active', 'superseded', 'deleted')),
                source TEXT NOT NULL DEFAULT 'legacy' CHECK(source IN ('legacy', 'user', 'agent', 'tool', 'import', 'compaction', 'system')),
                source_session_id TEXT,
                source_message_id TEXT,
                confidence REAL NOT NULL DEFAULT 1.0 CHECK(confidence >= 0.0 AND confidence <= 1.0),
                sensitivity TEXT NOT NULL DEFAULT 'normal' CHECK(sensitivity IN ('normal', 'sensitive', 'secret')),
                pinned INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0, 1)),
                supersedes_id TEXT,
                last_accessed_at TEXT,
                access_count INTEGER NOT NULL DEFAULT 0 CHECK(access_count >= 0),
                acl_scope TEXT NOT NULL DEFAULT 'owner'
                    CHECK(acl_scope IN ('owner', 'worker', 'group', 'conversation')),
                conversation_id TEXT,
                FOREIGN KEY(supersedes_id) REFERENCES agent_memories(id)
            );

            CREATE INDEX IF NOT EXISTS idx_agent_memories_type ON agent_memories(memory_type);
            CREATE INDEX IF NOT EXISTS idx_agent_memories_project ON agent_memories(project_dir);
            CREATE INDEX IF NOT EXISTS idx_agent_memories_user ON agent_memories(user_id);
            CREATE INDEX IF NOT EXISTS idx_agent_memories_active_scope
                ON agent_memories(status, user_id, project_dir, namespace, namespace_id);
            CREATE INDEX IF NOT EXISTS idx_agent_memories_canonical_key
                ON agent_memories(canonical_key, status);
            CREATE INDEX IF NOT EXISTS idx_agent_memories_acl
                ON agent_memories(status, user_id, namespace, namespace_id, acl_scope, conversation_id);

            CREATE TABLE IF NOT EXISTS agent_memory_revisions (
                id TEXT PRIMARY KEY,
                memory_id TEXT NOT NULL,
                revision INTEGER NOT NULL,
                event TEXT NOT NULL CHECK(event IN ('created', 'updated', 'superseded', 'deleted')),
                snapshot_json TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                FOREIGN KEY(memory_id) REFERENCES agent_memories(id) ON DELETE CASCADE,
                UNIQUE(memory_id, revision)
            );

            CREATE INDEX IF NOT EXISTS idx_agent_memory_revisions_memory
                ON agent_memory_revisions(memory_id, revision);
            "#,
        );
        Self { db }
    }

    /// Save a legacy free-form memory.
    ///
    /// The original signature remains intact. New callers that have a stable
    /// fact key and provenance should use [`Self::save_canonical`].
    pub fn save(
        &self,
        memory_type: MemoryType,
        title: &str,
        content: &str,
        project_dir: Option<&str>,
        user_id: Option<&str>,
    ) -> Result<AgentMemory> {
        let id = Uuid::new_v4().to_string();
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO agent_memories
                (id, memory_type, title, content, project_dir, user_id,
                 namespace, status, source, confidence, sensitivity, pinned, access_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'shared', 'active', 'legacy', 1.0, 'normal', 0, 0)",
            params![
                id,
                memory_type.as_str(),
                title,
                content,
                project_dir,
                user_id
            ],
        )?;

        let memory = load_memory_by_id(&tx, &id, true)?
            .ok_or_else(|| anyhow::anyhow!("memory not found after insert"))?;
        write_revision(&tx, &memory, MemoryRevisionEvent::Created)?;
        tx.commit()?;
        Ok(memory)
    }

    /// Save one canonical fact and atomically supersede the active fact with
    /// the same owner, project, namespace, namespace id, and canonical key.
    ///
    /// Exact semantic replays are idempotent. A changed fact creates a new row,
    /// points it at the prior row with `supersedes_id`, marks the prior row
    /// superseded, and records both lifecycle events in the revision ledger.
    pub fn save_canonical(&self, input: &CanonicalMemoryInput) -> Result<AgentMemory> {
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let memory = save_canonical_in_transaction(&tx, input)?;
        tx.commit()?;
        Ok(memory)
    }

    /// Soft-delete one active canonical fact in an exact owner and namespace
    /// scope.
    ///
    /// This is the only deletion shape used by governed learning. It cannot
    /// broaden from a missing project/user/namespace id into a wildcard, and
    /// it records the deleted snapshot in the immutable revision ledger.
    /// Replaying an already-applied tombstone is idempotent and returns `None`.
    pub fn tombstone_canonical_for_owner(
        &self,
        canonical_key: &str,
        project_dir: Option<&str>,
        user_id: Option<&str>,
        namespace: MemoryNamespace,
        namespace_id: Option<&str>,
    ) -> Result<Option<AgentMemory>> {
        let canonical_key = required_trimmed("canonical_key", canonical_key)?;
        let project_dir = optional_trimmed("project_dir", project_dir)?;
        let user_id = optional_trimmed("user_id", user_id)?;
        let namespace_id = optional_trimmed("namespace_id", namespace_id)?;
        if namespace == MemoryNamespace::Crew && namespace_id.is_none() {
            bail!("crew memory tombstones require namespace_id");
        }

        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let sql = format!(
            "SELECT {MEMORY_SELECT_COLUMNS}
             FROM agent_memories
             WHERE canonical_key = ?1
               AND project_dir IS ?2
               AND user_id IS ?3
               AND namespace = ?4
               AND namespace_id IS ?5
               AND status = 'active'
             LIMIT 1"
        );
        let existing = tx
            .query_row(
                &sql,
                params![
                    canonical_key,
                    project_dir,
                    user_id,
                    namespace.as_str(),
                    namespace_id
                ],
                |row| Ok(row_to_memory(row)),
            )
            .optional()?;
        let Some(existing) = existing else {
            tx.commit()?;
            return Ok(None);
        };

        soft_delete(&tx, &existing.id, Some(user_id))?;
        let deleted = load_memory_for_owner(&tx, &existing.id, user_id, false)?
            .ok_or_else(|| anyhow::anyhow!("canonical memory disappeared after tombstone"))?;
        write_revision(&tx, &deleted, MemoryRevisionEvent::Deleted)?;
        tx.commit()?;
        Ok(Some(deleted))
    }

    /// Get an active memory by id without applying an owner boundary.
    ///
    /// Retained for legacy callers. Request-facing code should use
    /// [`Self::get_for_owner`].
    pub fn get(&self, id: &str) -> Result<Option<AgentMemory>> {
        Ok(load_memory_by_id(self.db.conn(), id, true)?)
    }

    /// Get an active memory only when it belongs to the exact owner.
    ///
    /// `None` means the local/global owner and never matches user-owned rows.
    pub fn get_for_owner(&self, id: &str, user_id: Option<&str>) -> Result<Option<AgentMemory>> {
        Ok(load_memory_for_owner(self.db.conn(), id, user_id, true)?)
    }

    /// Update an active memory without applying an owner boundary.
    ///
    /// Retained for legacy callers. The mutation is still revisioned and will
    /// not revive deleted or superseded rows.
    pub fn update(&self, id: &str, title: Option<&str>, content: Option<&str>) -> Result<()> {
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let Some(existing) = load_memory_by_id(&tx, id, true)? else {
            tx.commit()?;
            return Ok(());
        };
        if title.is_none() && content.is_none() {
            tx.commit()?;
            return Ok(());
        }

        update_memory_fields(&tx, id, title, content, None)?;
        let updated = load_memory_by_id(&tx, id, true)?
            .ok_or_else(|| anyhow::anyhow!("memory not found after update"))?;
        if updated != existing {
            write_revision(&tx, &updated, MemoryRevisionEvent::Updated)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Update an active memory only for the exact owner.
    ///
    /// Returns `None` for both missing memories and owner mismatches so callers
    /// cannot use this API to probe another owner's ids.
    pub fn update_for_owner(
        &self,
        id: &str,
        user_id: Option<&str>,
        title: Option<&str>,
        content: Option<&str>,
    ) -> Result<Option<AgentMemory>> {
        validate_optional_content("title", title)?;
        validate_optional_content("content", content)?;

        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let Some(existing) = load_memory_for_owner(&tx, id, user_id, true)? else {
            tx.commit()?;
            return Ok(None);
        };
        if title.is_none() && content.is_none() {
            tx.commit()?;
            return Ok(Some(existing));
        }

        update_memory_fields(&tx, id, title, content, Some(user_id))?;
        let updated = load_memory_for_owner(&tx, id, user_id, true)?
            .ok_or_else(|| anyhow::anyhow!("memory not found after owner-scoped update"))?;
        if updated != existing {
            write_revision(&tx, &updated, MemoryRevisionEvent::Updated)?;
        }
        tx.commit()?;
        Ok(Some(updated))
    }

    /// Soft-delete an active memory without applying an owner boundary.
    ///
    /// Active reads no longer return the row, while provenance and revision
    /// history remain recoverable.
    pub fn delete(&self, id: &str) -> Result<()> {
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let Some(_) = load_memory_by_id(&tx, id, true)? else {
            tx.commit()?;
            return Ok(());
        };
        soft_delete(&tx, id, None)?;
        let deleted = load_memory_by_id(&tx, id, false)?
            .ok_or_else(|| anyhow::anyhow!("memory not found after delete"))?;
        write_revision(&tx, &deleted, MemoryRevisionEvent::Deleted)?;
        tx.commit()?;
        Ok(())
    }

    /// Soft-delete an active memory only for the exact owner.
    pub fn delete_for_owner(&self, id: &str, user_id: Option<&str>) -> Result<bool> {
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        if load_memory_for_owner(&tx, id, user_id, true)?.is_none() {
            tx.commit()?;
            return Ok(false);
        }

        soft_delete(&tx, id, Some(user_id))?;
        let deleted = load_memory_for_owner(&tx, id, user_id, false)?
            .ok_or_else(|| anyhow::anyhow!("memory not found after owner-scoped delete"))?;
        write_revision(&tx, &deleted, MemoryRevisionEvent::Deleted)?;
        tx.commit()?;
        Ok(true)
    }

    /// Record successful retrieval without coupling access telemetry to every
    /// low-level read.
    pub fn record_access_for_owner(
        &self,
        id: &str,
        user_id: Option<&str>,
    ) -> Result<Option<AgentMemory>> {
        self.db.conn().execute(
            "UPDATE agent_memories
             SET last_accessed_at = datetime('now'), access_count = access_count + 1
             WHERE id = ?1 AND status = 'active' AND user_id IS ?2",
            params![id, user_id],
        )?;
        self.get_for_owner(id, user_id)
    }

    /// Return the immutable lifecycle ledger for a memory owned by the exact
    /// user scope. Deleted and superseded memories remain auditable here.
    pub fn list_revisions_for_owner(
        &self,
        memory_id: &str,
        user_id: Option<&str>,
    ) -> Result<Vec<AgentMemoryRevision>> {
        if load_memory_for_owner(self.db.conn(), memory_id, user_id, false)?.is_none() {
            return Ok(Vec::new());
        }

        let mut stmt = self.db.conn().prepare(
            "SELECT id, memory_id, revision, event, snapshot_json, created_at
             FROM agent_memory_revisions
             WHERE memory_id = ?1
             ORDER BY revision",
        )?;
        let rows = stmt.query_map([memory_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?;

        let mut revisions = Vec::new();
        for row in rows {
            let (id, memory_id, revision, event, snapshot_json, created_at) = row?;
            revisions.push(AgentMemoryRevision {
                id,
                memory_id,
                revision,
                event: event
                    .parse()
                    .map_err(|error: String| anyhow::anyhow!(error))?,
                snapshot: serde_json::from_str(&snapshot_json)
                    .context("decode agent memory revision snapshot")?,
                created_at,
            });
        }
        Ok(revisions)
    }

    /// List active memories, optionally filtered by project and owner-visible
    /// scope. When `project_dir` is set, project-scoped and global rows are
    /// returned. When `user_id` is set, user-owned and global rows are returned.
    pub fn list(&self, project_dir: Option<&str>, user_id: Option<&str>) -> Vec<AgentMemory> {
        let (sql, bound) = build_list_query(None, project_dir, user_id);
        self.query_memories(&sql, &bound)
    }

    /// List active memories for one exact owner while retaining the existing
    /// project visibility rule (project-specific plus owner-global rows).
    ///
    /// This is the Hive prompt/snapshot boundary. Unlike [`Self::list`], an
    /// authenticated owner never inherits legacy `user_id IS NULL` memories,
    /// and local mode never sees authenticated users' rows.
    pub fn list_for_exact_owner(
        &self,
        project_dir: Option<&str>,
        user_id: Option<&str>,
    ) -> Vec<AgentMemory> {
        let mut sql =
            format!("SELECT {MEMORY_SELECT_COLUMNS} FROM agent_memories WHERE status = 'active'");
        let mut bound = Vec::new();

        if let Some(project_dir) = project_dir {
            bound.push(project_dir.to_string());
            sql.push_str(&format!(
                " AND (project_dir = ?{} OR project_dir IS NULL)",
                bound.len()
            ));
        } else {
            sql.push_str(" AND project_dir IS NULL");
        }

        if let Some(user_id) = user_id {
            bound.push(user_id.to_string());
            sql.push_str(&format!(" AND user_id = ?{}", bound.len()));
        } else {
            sql.push_str(" AND user_id IS NULL");
        }

        sql.push_str(" ORDER BY updated_at DESC");
        self.query_memories(&sql, &bound)
    }

    /// Lower confidence of unpinned active memories that have not been
    /// updated recently. Rows are never deleted; confidence is clamped to
    /// `floor` so stale facts remain recoverable.
    pub fn decay_stale(&self, older_than_days: i64, factor: f64, floor: f64) -> Result<usize> {
        let older_than_days = older_than_days.max(1);
        let factor = factor.clamp(0.0, 1.0);
        let floor = floor.clamp(0.0, 1.0);
        let cutoff = (Utc::now() - Duration::days(older_than_days)).to_rfc3339();
        let changed = self.db.conn().execute(
            "UPDATE agent_memories
             SET confidence = MAX(?1, confidence * ?2)
             WHERE pinned = 0
               AND status = 'active'
               AND updated_at < ?3",
            params![floor, factor, cutoff],
        )?;
        Ok(changed)
    }

    /// Hive prompt boundary: exact owner, no secret injection, and ACL
    /// scopes so one Worker cannot read another Worker's private facts
    /// even when both sit in the same group room.
    pub fn list_for_hive_reader(&self, reader: &HiveMemoryReader<'_>) -> Vec<AgentMemory> {
        let mut memories = self.list_for_exact_owner(reader.project_dir, reader.user_id);
        memories.retain(|memory| memory_visible_to_hive_reader(memory, reader));
        memories
    }

    /// Prompt/tool boundary for ordinary Chat and Code sessions.
    ///
    /// Standard sessions may consume only owner-scoped Shared memory. Hive,
    /// Worker, group, and conversation namespaces are deliberately not
    /// inherited merely because the same user owns both sessions.
    pub fn list_for_standard_reader(
        &self,
        project_dir: Option<&str>,
        user_id: Option<&str>,
    ) -> Vec<AgentMemory> {
        let mut memories = self.list(project_dir, user_id);
        memories.retain(memory_visible_to_standard_reader);
        memories
    }

    /// Whether one already-loaded memory is visible to a Hive reader.
    /// Used by the memory tool so a Worker cannot mutate a guessed id that
    /// belongs to another Worker's private namespace.
    pub fn visible_to_hive_reader(memory: &AgentMemory, reader: &HiveMemoryReader<'_>) -> bool {
        memory_visible_to_hive_reader(memory, reader)
    }

    /// Whether a standard Chat/Code reader may see or mutate an already
    /// loaded memory. This is also used to make guessed ids fail closed.
    pub fn visible_to_standard_reader(memory: &AgentMemory) -> bool {
        memory_visible_to_standard_reader(memory)
    }

    /// List active memories of a specific type.
    pub fn list_by_type(
        &self,
        memory_type: MemoryType,
        project_dir: Option<&str>,
        user_id: Option<&str>,
    ) -> Vec<AgentMemory> {
        let (sql, bound) = build_list_query(Some(memory_type), project_dir, user_id);
        self.query_memories(&sql, &bound)
    }

    /// Find an active memory by exact title within a project scope.
    pub fn find_by_title(&self, title: &str, project_dir: Option<&str>) -> Option<AgentMemory> {
        let mut sql = format!(
            "SELECT {MEMORY_SELECT_COLUMNS} FROM agent_memories
             WHERE title = ?1 AND status = 'active'"
        );
        let mut bound = vec![title.to_string()];
        if let Some(project_dir) = project_dir {
            bound.push(project_dir.to_string());
            sql.push_str(&format!(
                " AND (project_dir = ?{} OR project_dir IS NULL)",
                bound.len()
            ));
        } else {
            sql.push_str(" AND project_dir IS NULL");
        }
        sql.push_str(" ORDER BY updated_at DESC LIMIT 1");
        self.query_one(&sql, &bound)
    }

    /// Find an active memory by exact title within project/user-visible scope.
    pub fn find_by_title_for_user(
        &self,
        title: &str,
        project_dir: Option<&str>,
        user_id: Option<&str>,
    ) -> Option<AgentMemory> {
        let mut sql = format!(
            "SELECT {MEMORY_SELECT_COLUMNS} FROM agent_memories
             WHERE title = ?1 AND status = 'active'"
        );
        let mut bound = vec![title.to_string()];
        add_visible_scope(&mut sql, &mut bound, project_dir, "project_dir");
        add_visible_scope(&mut sql, &mut bound, user_id, "user_id");
        sql.push_str(" ORDER BY updated_at DESC LIMIT 1");
        self.query_one(&sql, &bound)
    }

    /// Save a legacy memory or update an active exact-title match in the same
    /// exact project/user scope.
    pub fn save_or_update_by_title(
        &self,
        memory_type: MemoryType,
        title: &str,
        content: &str,
        project_dir: Option<&str>,
        user_id: Option<&str>,
    ) -> Result<(AgentMemory, bool)> {
        if let Some(existing) = self.find_by_title_in_exact_scope(title, project_dir, user_id) {
            self.update(&existing.id, Some(title), Some(content))?;
            let updated = self
                .get(&existing.id)?
                .ok_or_else(|| anyhow::anyhow!("memory not found after update"))?;
            return Ok((updated, false));
        }

        let created = self.save(memory_type, title, content, project_dir, user_id)?;
        Ok((created, true))
    }

    pub fn find_by_title_in_exact_scope(
        &self,
        title: &str,
        project_dir: Option<&str>,
        user_id: Option<&str>,
    ) -> Option<AgentMemory> {
        let mut sql = format!(
            "SELECT {MEMORY_SELECT_COLUMNS} FROM agent_memories
             WHERE title = ?1 AND status = 'active'"
        );
        let mut bound = vec![title.to_string()];
        add_exact_scope(&mut sql, &mut bound, project_dir, "project_dir");
        add_exact_scope(&mut sql, &mut bound, user_id, "user_id");
        sql.push_str(" ORDER BY updated_at DESC LIMIT 1");
        self.query_one(&sql, &bound)
    }

    fn query_one(&self, sql: &str, bound: &[String]) -> Option<AgentMemory> {
        let mut stmt = self.db.conn().prepare(sql).ok()?;
        let params = to_sql_params(bound);
        stmt.query_row(params.as_slice(), |row| Ok(row_to_memory(row)))
            .ok()
    }

    fn query_memories(&self, sql: &str, bound: &[String]) -> Vec<AgentMemory> {
        let mut stmt = match self.db.conn().prepare(sql) {
            Ok(statement) => statement,
            Err(error) => {
                tracing::warn!("Memory query failed (prepare): {}", error);
                return Vec::new();
            }
        };
        let params = to_sql_params(bound);
        let rows = match stmt.query_map(params.as_slice(), |row| Ok(row_to_memory(row))) {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!("Memory query failed (execute): {}", error);
                return Vec::new();
            }
        };
        rows.filter_map(|row| row.ok()).collect()
    }
}

/// Save canonical memory inside an existing immediate transaction.
///
/// This is crate-private so higher-level governed workflows can atomically
/// commit a canonical memory and their own review state without duplicating
/// the memory invariants or exposing a transaction-bearing API publicly.
pub(crate) fn save_canonical_in_transaction(
    tx: &Transaction<'_>,
    input: &CanonicalMemoryInput,
) -> Result<AgentMemory> {
    let input = NormalizedCanonicalInput::new(input)?;
    let existing = find_active_canonical(tx, &input)?;

    if let Some(existing) = existing.as_ref() {
        if canonical_matches(existing, &input) {
            return Ok(existing.clone());
        }
    }

    let supersedes_id = if let Some(existing) = existing {
        tx.execute(
            "UPDATE agent_memories
             SET status = 'superseded', updated_at = datetime('now')
             WHERE id = ?1 AND status = 'active'",
            [&existing.id],
        )?;
        let superseded = load_memory_by_id(tx, &existing.id, false)?
            .ok_or_else(|| anyhow::anyhow!("superseded memory disappeared"))?;
        write_revision(tx, &superseded, MemoryRevisionEvent::Superseded)?;
        Some(existing.id)
    } else {
        None
    };

    let id = Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO agent_memories
            (id, memory_type, title, content, project_dir, user_id,
             canonical_key, namespace, namespace_id, status, source,
             source_session_id, source_message_id, confidence, sensitivity,
             pinned, supersedes_id, access_count, acl_scope, conversation_id)
         VALUES
            (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'active', ?10,
             ?11, ?12, ?13, ?14, ?15, ?16, 0, ?17, ?18)",
        params![
            id,
            input.memory_type.as_str(),
            input.title,
            input.content,
            input.project_dir,
            input.user_id,
            input.canonical_key,
            input.namespace.as_str(),
            input.namespace_id,
            input.source.as_str(),
            input.source_session_id,
            input.source_message_id,
            input.confidence,
            input.sensitivity.as_str(),
            input.pinned,
            supersedes_id,
            input.acl_scope.as_str(),
            input.conversation_id,
        ],
    )?;

    let memory = load_memory_by_id(tx, &id, true)?
        .ok_or_else(|| anyhow::anyhow!("canonical memory not found after insert"))?;
    write_revision(tx, &memory, MemoryRevisionEvent::Created)?;
    Ok(memory)
}

pub(crate) fn load_canonical_for_provenance_from_connection(
    conn: &rusqlite::Connection,
    input: &CanonicalMemoryInput,
) -> Result<Option<AgentMemory>> {
    let input = NormalizedCanonicalInput::new(input)?;
    if input.source_session_id.is_none() || input.source_message_id.is_none() {
        bail!("canonical provenance lookup requires session and message ids");
    }
    let sql = format!(
        "SELECT {MEMORY_SELECT_COLUMNS}
         FROM agent_memories
         WHERE canonical_key = ?1
           AND project_dir IS ?2
           AND user_id IS ?3
           AND namespace = ?4
           AND namespace_id IS ?5
           AND source = ?6
           AND source_session_id IS ?7
           AND source_message_id IS ?8
         ORDER BY created_at ASC
         LIMIT 1"
    );
    Ok(conn
        .query_row(
            &sql,
            params![
                input.canonical_key,
                input.project_dir,
                input.user_id,
                input.namespace.as_str(),
                input.namespace_id,
                input.source.as_str(),
                input.source_session_id,
                input.source_message_id,
            ],
            |row| Ok(row_to_memory(row)),
        )
        .optional()?)
}

struct NormalizedCanonicalInput<'a> {
    memory_type: MemoryType,
    canonical_key: &'a str,
    title: &'a str,
    content: &'a str,
    project_dir: Option<&'a str>,
    user_id: Option<&'a str>,
    namespace: MemoryNamespace,
    namespace_id: Option<&'a str>,
    source: super::model::MemorySource,
    source_session_id: Option<&'a str>,
    source_message_id: Option<&'a str>,
    confidence: f64,
    sensitivity: super::model::MemorySensitivity,
    pinned: bool,
    acl_scope: MemoryAclScope,
    conversation_id: Option<&'a str>,
}

impl<'a> NormalizedCanonicalInput<'a> {
    fn new(input: &'a CanonicalMemoryInput) -> Result<Self> {
        let canonical_key = required_trimmed("canonical_key", &input.canonical_key)?;
        let title = required_trimmed("title", &input.title)?;
        let content = required_trimmed("content", &input.content)?;
        let project_dir = optional_trimmed("project_dir", input.project_dir.as_deref())?;
        let user_id = optional_trimmed("user_id", input.user_id.as_deref())?;
        let namespace_id = optional_trimmed("namespace_id", input.namespace_id.as_deref())?;
        let source_session_id =
            optional_trimmed("source_session_id", input.source_session_id.as_deref())?;
        let source_message_id =
            optional_trimmed("source_message_id", input.source_message_id.as_deref())?;
        let conversation_id =
            optional_trimmed("conversation_id", input.conversation_id.as_deref())?;
        if input.namespace == MemoryNamespace::Crew && namespace_id.is_none() {
            bail!("crew memories require namespace_id");
        }
        if !input.confidence.is_finite() || !(0.0..=1.0).contains(&input.confidence) {
            bail!("memory confidence must be between 0.0 and 1.0");
        }
        // Crew rows default to Worker ACL so a group member run cannot
        // inherit another Worker's private facts through the Owner lane.
        let acl_scope = if input.namespace == MemoryNamespace::Crew
            && input.acl_scope == MemoryAclScope::Owner
        {
            MemoryAclScope::Worker
        } else {
            input.acl_scope
        };

        Ok(Self {
            memory_type: input.memory_type,
            canonical_key,
            title,
            content,
            project_dir,
            user_id,
            namespace: input.namespace,
            namespace_id,
            source: input.source,
            source_session_id,
            source_message_id,
            confidence: input.confidence,
            sensitivity: input.sensitivity,
            pinned: input.pinned,
            acl_scope,
            conversation_id,
        })
    }
}

fn memory_visible_to_hive_reader(memory: &AgentMemory, reader: &HiveMemoryReader<'_>) -> bool {
    if memory.sensitivity == MemorySensitivity::Secret {
        return false;
    }
    match memory.acl_scope {
        MemoryAclScope::Owner => match reader.worker_namespace_id {
            Some(_) => memory.namespace == MemoryNamespace::Shared,
            None => {
                memory.namespace == MemoryNamespace::Shared
                    || memory.namespace == MemoryNamespace::Hive
            }
        },
        MemoryAclScope::Worker => {
            memory.namespace == MemoryNamespace::Crew
                && reader.worker_namespace_id.is_some()
                && memory.namespace_id.as_deref() == reader.worker_namespace_id
        }
        MemoryAclScope::Group => {
            reader.group_id.is_some() && memory.conversation_id.as_deref() == reader.group_id
        }
        MemoryAclScope::Conversation => {
            reader.conversation_id.is_some()
                && memory.conversation_id.as_deref() == reader.conversation_id
        }
    }
}

fn memory_visible_to_standard_reader(memory: &AgentMemory) -> bool {
    memory.sensitivity != MemorySensitivity::Secret
        && memory.acl_scope == MemoryAclScope::Owner
        && memory.namespace == MemoryNamespace::Shared
}

fn required_trimmed<'a>(field: &str, value: &'a str) -> Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        bail!("memory {field} must not be empty");
    }
    Ok(value)
}

fn optional_trimmed<'a>(field: &str, value: Option<&'a str>) -> Result<Option<&'a str>> {
    match value {
        Some(value) => Ok(Some(required_trimmed(field, value)?)),
        None => Ok(None),
    }
}

fn validate_optional_content(field: &str, value: Option<&str>) -> Result<()> {
    if let Some(value) = value {
        required_trimmed(field, value)?;
    }
    Ok(())
}

fn canonical_matches(memory: &AgentMemory, input: &NormalizedCanonicalInput<'_>) -> bool {
    memory.memory_type == input.memory_type
        && memory.canonical_key.as_deref() == Some(input.canonical_key)
        && memory.title == input.title
        && memory.content == input.content
        && memory.project_dir.as_deref() == input.project_dir
        && memory.user_id.as_deref() == input.user_id
        && memory.namespace == input.namespace
        && memory.namespace_id.as_deref() == input.namespace_id
        && memory.source == input.source
        && memory.source_session_id.as_deref() == input.source_session_id
        && memory.source_message_id.as_deref() == input.source_message_id
        && memory.confidence == input.confidence
        && memory.sensitivity == input.sensitivity
        && memory.pinned == input.pinned
        && memory.acl_scope == input.acl_scope
        && memory.conversation_id.as_deref() == input.conversation_id
}

fn find_active_canonical(
    tx: &Transaction<'_>,
    input: &NormalizedCanonicalInput<'_>,
) -> Result<Option<AgentMemory>> {
    let sql = format!(
        "SELECT {MEMORY_SELECT_COLUMNS}
         FROM agent_memories
         WHERE canonical_key = ?1
           AND namespace = ?2
           AND project_dir IS ?3
           AND user_id IS ?4
           AND namespace_id IS ?5
           AND status = 'active'
         ORDER BY updated_at DESC
         LIMIT 1"
    );
    Ok(tx
        .query_row(
            &sql,
            params![
                input.canonical_key,
                input.namespace.as_str(),
                input.project_dir,
                input.user_id,
                input.namespace_id,
            ],
            |row| Ok(row_to_memory(row)),
        )
        .optional()?)
}

fn load_memory_by_id(
    conn: &rusqlite::Connection,
    id: &str,
    active_only: bool,
) -> rusqlite::Result<Option<AgentMemory>> {
    let status = if active_only {
        " AND status = 'active'"
    } else {
        ""
    };
    let sql = format!("SELECT {MEMORY_SELECT_COLUMNS} FROM agent_memories WHERE id = ?1{status}");
    conn.query_row(&sql, [id], |row| Ok(row_to_memory(row)))
        .optional()
}

fn load_memory_for_owner(
    conn: &rusqlite::Connection,
    id: &str,
    user_id: Option<&str>,
    active_only: bool,
) -> rusqlite::Result<Option<AgentMemory>> {
    let status = if active_only {
        " AND status = 'active'"
    } else {
        ""
    };
    let sql = format!(
        "SELECT {MEMORY_SELECT_COLUMNS}
         FROM agent_memories
         WHERE id = ?1 AND user_id IS ?2{status}"
    );
    conn.query_row(&sql, params![id, user_id], |row| Ok(row_to_memory(row)))
        .optional()
}

fn update_memory_fields(
    tx: &Transaction<'_>,
    id: &str,
    title: Option<&str>,
    content: Option<&str>,
    owner: Option<Option<&str>>,
) -> Result<()> {
    if let Some(user_id) = owner {
        tx.execute(
            "UPDATE agent_memories
             SET title = COALESCE(?1, title),
                 content = COALESCE(?2, content),
                 updated_at = datetime('now')
             WHERE id = ?3 AND status = 'active' AND user_id IS ?4",
            params![title, content, id, user_id],
        )?;
    } else {
        tx.execute(
            "UPDATE agent_memories
             SET title = COALESCE(?1, title),
                 content = COALESCE(?2, content),
                 updated_at = datetime('now')
             WHERE id = ?3 AND status = 'active'",
            params![title, content, id],
        )?;
    }
    Ok(())
}

fn soft_delete(tx: &Transaction<'_>, id: &str, owner: Option<Option<&str>>) -> Result<()> {
    if let Some(user_id) = owner {
        tx.execute(
            "UPDATE agent_memories
             SET status = 'deleted', updated_at = datetime('now')
             WHERE id = ?1 AND status = 'active' AND user_id IS ?2",
            params![id, user_id],
        )?;
    } else {
        tx.execute(
            "UPDATE agent_memories
             SET status = 'deleted', updated_at = datetime('now')
             WHERE id = ?1 AND status = 'active'",
            [id],
        )?;
    }
    Ok(())
}

fn write_revision(
    tx: &Transaction<'_>,
    memory: &AgentMemory,
    event: MemoryRevisionEvent,
) -> Result<()> {
    let revision = tx.query_row(
        "SELECT COALESCE(MAX(revision), 0) + 1
         FROM agent_memory_revisions
         WHERE memory_id = ?1",
        [&memory.id],
        |row| row.get::<_, i64>(0),
    )?;
    let snapshot_json = serde_json::to_string(memory).context("encode agent memory revision")?;
    tx.execute(
        "INSERT INTO agent_memory_revisions
            (id, memory_id, revision, event, snapshot_json)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            Uuid::new_v4().to_string(),
            memory.id,
            revision,
            event.as_str(),
            snapshot_json,
        ],
    )?;
    Ok(())
}

fn add_visible_scope(sql: &mut String, bound: &mut Vec<String>, value: Option<&str>, column: &str) {
    if let Some(value) = value {
        bound.push(value.to_string());
        sql.push_str(&format!(
            " AND ({column} = ?{} OR {column} IS NULL)",
            bound.len()
        ));
    } else {
        sql.push_str(&format!(" AND {column} IS NULL"));
    }
}

fn add_exact_scope(sql: &mut String, bound: &mut Vec<String>, value: Option<&str>, column: &str) {
    if let Some(value) = value {
        bound.push(value.to_string());
        sql.push_str(&format!(" AND {column} = ?{}", bound.len()));
    } else {
        sql.push_str(&format!(" AND {column} IS NULL"));
    }
}

fn to_sql_params(bound: &[String]) -> Vec<&dyn rusqlite::types::ToSql> {
    bound
        .iter()
        .map(|value| value as &dyn rusqlite::types::ToSql)
        .collect()
}
