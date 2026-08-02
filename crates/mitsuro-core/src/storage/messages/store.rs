use std::path::Path;

use anyhow::{ensure, Context, Result};
use chrono::Utc;
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};

use crate::storage::database::Database;
use crate::storage::episodes::EpisodeStore;

use super::model::StoredMessageRecord;

const CHILD_WAKE_PENDING_PREFIX: &str = "child-wake-";

pub struct MessageStore<'a> {
    db: &'a Database,
}

impl<'a> MessageStore<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn save_message(&self, session_id: &str, role: &str, content_json: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        self.db.conn().execute(
            "INSERT INTO messages (session_id, role, content, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![session_id, role, content_json, now],
        )?;
        let message_id = self.db.conn().last_insert_rowid();

        if let Err(error) = EpisodeStore::new(self.db).record_message(
            session_id,
            message_id,
            role,
            content_json,
            &now,
        ) {
            tracing::warn!(
                session_id,
                message_id,
                error = %error,
                "Canonical message saved but episodic recall indexing failed"
            );
        }

        self.db.conn().execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;

        Ok(())
    }

    /// Durably queue a live steering message without exposing it as canonical
    /// model history until the active run reaches a safe boundary.
    pub fn queue_pending_steering(
        &self,
        session_id: &str,
        pending_id: &str,
        content_json: &str,
    ) -> Result<()> {
        self.save_message(
            session_id,
            &format!("pending_user:{pending_id}"),
            content_json,
        )
    }

    /// Atomically queue one durable steering input and retain a compact
    /// receipt after promotion. Returns `false` when this session has already
    /// accepted the same pending ID, including after the pending row became a
    /// canonical user message.
    pub fn queue_pending_steering_once(
        &self,
        session_id: &str,
        pending_id: &str,
        content_json: &str,
    ) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO steering_idempotency (session_id, pending_id, created_at)
             VALUES (?1, ?2, ?3)",
            params![session_id, pending_id, now],
        )?;
        if inserted == 0 {
            tx.commit()?;
            return Ok(false);
        }

        tx.execute(
            "INSERT INTO messages (session_id, role, content, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                session_id,
                format!("pending_user:{pending_id}"),
                content_json,
                now
            ],
        )?;
        tx.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;
        tx.commit()?;

        Ok(true)
    }

    pub fn has_pending_steering(&self, session_id: &str, pending_id: &str) -> Result<bool> {
        Ok(self
            .load_pending_steering(session_id, pending_id)?
            .is_some())
    }

    pub fn load_pending_steering(
        &self,
        session_id: &str,
        pending_id: &str,
    ) -> Result<Option<String>> {
        let role = format!("pending_user:{pending_id}");
        self.db
            .conn()
            .query_row(
                "SELECT content FROM messages
                 WHERE session_id = ?1 AND role = ?2
                 LIMIT 1",
                params![session_id, role],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    /// Atomically move a durable steering message to the end of canonical
    /// user history. Returning `None` makes duplicate delivery idempotent.
    pub fn promote_pending_steering(
        &self,
        session_id: &str,
        pending_id: &str,
    ) -> Result<Option<String>> {
        ensure!(
            !pending_id.starts_with(CHILD_WAKE_PENDING_PREFIX),
            "child completion steering requires workspace-authorized promotion"
        );
        self.promote_pending_steering_inner(session_id, pending_id, None)
    }

    /// Promote a child completion only while its durable launch workspace is
    /// still the parent session's active project. The workspace comparison is
    /// performed inside the same immediate transaction as canonicalization so
    /// a concurrent project switch cannot race the authority check.
    pub(crate) fn promote_pending_child_wake(
        &self,
        session_id: &str,
        pending_id: &str,
        launch_workspace: &Path,
    ) -> Result<Option<String>> {
        ensure!(
            pending_id.starts_with(CHILD_WAKE_PENDING_PREFIX),
            "workspace-authorized promotion is reserved for child completions"
        );
        self.promote_pending_steering_inner(session_id, pending_id, Some(launch_workspace))
    }

    fn promote_pending_steering_inner(
        &self,
        session_id: &str,
        pending_id: &str,
        launch_workspace: Option<&Path>,
    ) -> Result<Option<String>> {
        let role = format!("pending_user:{pending_id}");
        let now = Utc::now().to_rfc3339();
        // Reserve the writer before reading the pending row. With a deferred
        // transaction another connection can commit after this read, making
        // the read snapshot impossible to upgrade and surfacing an immediate
        // SQLITE_BUSY_SNAPSHOT despite the connection busy timeout.
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let content = tx
            .query_row(
                "SELECT content FROM messages WHERE session_id = ?1 AND role = ?2 LIMIT 1",
                params![session_id, role],
                |row| row.get::<_, String>(0),
            )
            .optional()?;

        let Some(content) = content else {
            tx.commit()?;
            return Ok(None);
        };

        if let Some(launch_workspace) = launch_workspace {
            let (project_dir, working_dir, session_type) = tx
                .query_row(
                    "SELECT project_dir, working_dir, session_type
                       FROM sessions
                      WHERE id = ?1",
                    [session_id],
                    |row| {
                        Ok((
                            row.get::<_, Option<String>>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()?
                .context("child completion parent session no longer exists")?;
            ensure!(
                matches!(session_type.as_str(), "chat" | "code"),
                "child completion cannot promote into a Hive-owned session"
            );
            let current_workspace = project_dir
                .as_deref()
                .or(working_dir.as_deref())
                .context("child completion parent session has no current project workspace")?;
            let current_workspace = Path::new(current_workspace)
                .canonicalize()
                .context("canonicalizing child completion parent workspace")?;
            let launch_workspace = launch_workspace
                .canonicalize()
                .context("canonicalizing child completion launch workspace")?;
            ensure!(
                current_workspace == launch_workspace,
                "child completion parent session project no longer matches its durable launch workspace"
            );
        }

        tx.execute(
            "DELETE FROM messages WHERE session_id = ?1 AND role = ?2",
            params![session_id, role],
        )?;
        tx.execute(
            "INSERT INTO messages (session_id, role, content, created_at)
             VALUES (?1, 'user', ?2, ?3)",
            params![session_id, content, now],
        )?;
        let promoted_message_id = tx.last_insert_rowid();
        tx.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;
        tx.commit()?;

        if let Err(error) = EpisodeStore::new(self.db).record_message(
            session_id,
            promoted_message_id,
            "user",
            &content,
            &now,
        ) {
            tracing::warn!(
                session_id,
                message_id = promoted_message_id,
                error = %error,
                "Promoted steering saved but episodic recall indexing failed"
            );
        }

        Ok(Some(content))
    }

    /// Recover steering accepted by a run that exited before it could reach a
    /// safe boundary. Callers must hold the session run lock. Pending messages
    /// are moved to the canonical tail because the interrupted run may have
    /// persisted its final assistant message after the steering was staged.
    /// Child completions are intentionally excluded: they carry a durable
    /// launch-workspace authority that ordinary chat recovery cannot approve.
    /// The child-wake controller promotes them through the guarded path above.
    pub fn promote_orphaned_pending_steering(&self, session_id: &str) -> Result<usize> {
        let now = Utc::now().to_rfc3339();
        // This read-then-write sequence must reserve the writer up front; see
        // `promote_pending_steering` for the WAL snapshot-upgrade race.
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let pending = {
            let mut stmt = tx.prepare(
                "SELECT content FROM messages
                 WHERE session_id = ?1
                   AND role LIKE 'pending_user:%'
                   AND role NOT LIKE 'pending_user:child-wake-%'
                 ORDER BY id",
            )?;
            let rows = stmt.query_map([session_id], |row| row.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        if pending.is_empty() {
            tx.commit()?;
            return Ok(0);
        }

        tx.execute(
            "DELETE FROM messages
             WHERE session_id = ?1
               AND role LIKE 'pending_user:%'
               AND role NOT LIKE 'pending_user:child-wake-%'",
            [session_id],
        )?;
        let mut promoted = Vec::with_capacity(pending.len());
        for content in &pending {
            tx.execute(
                "INSERT INTO messages (session_id, role, content, created_at)
                 VALUES (?1, 'user', ?2, ?3)",
                params![session_id, content, now],
            )?;
            promoted.push((tx.last_insert_rowid(), content.clone()));
        }
        tx.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;
        tx.commit()?;

        for (message_id, content) in promoted {
            if let Err(error) = EpisodeStore::new(self.db)
                .record_message(session_id, message_id, "user", &content, &now)
            {
                tracing::warn!(
                    session_id,
                    message_id,
                    error = %error,
                    "Recovered steering saved but episodic recall indexing failed"
                );
            }
        }

        Ok(pending.len())
    }

    pub fn replace_session_messages(
        &self,
        session_id: &str,
        messages: &[(String, String)],
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        // Preserve pending rows and replace canonical history from one stable
        // writer snapshot. A deferred transaction can lose its write upgrade
        // when the scheduler commits between the SELECT and DELETE.
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;

        let pending = {
            let mut stmt = tx.prepare(
                "SELECT role, content, created_at FROM messages
                 WHERE session_id = ?1 AND role LIKE 'pending_user:%'
                 ORDER BY id",
            )?;
            let rows = stmt.query_map([session_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        tx.execute("DELETE FROM messages WHERE session_id = ?1", [session_id])?;

        for (role, content_json) in messages {
            tx.execute(
                "INSERT INTO messages (session_id, role, content, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![session_id, role, content_json, now],
            )?;
        }

        // Staged steering remains non-canonical and follows the replaced
        // history. Its eventual promotion will therefore append at the exact
        // next safe boundary instead of leaking into compacted history.
        for (role, content_json, created_at) in pending {
            tx.execute(
                "INSERT INTO messages (session_id, role, content, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![session_id, role, content_json, created_at],
            )?;
        }

        tx.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;
        tx.commit()?;

        Ok(())
    }

    pub fn load_session_messages(&self, session_id: &str) -> Result<Vec<(String, String)>> {
        self.load_session_messages_paginated(session_id, 0, None)
    }

    pub fn load_session_message_records(
        &self,
        session_id: &str,
    ) -> Result<Vec<StoredMessageRecord>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, role, content, created_at
             FROM messages
             WHERE session_id = ?1 AND role NOT LIKE 'pending_user:%'
             ORDER BY id",
        )?;

        let rows = stmt.query_map([session_id], |row| {
            Ok(StoredMessageRecord {
                id: row.get(0)?,
                role: row.get(1)?,
                content_json: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn load_session_messages_paginated(
        &self,
        session_id: &str,
        offset: usize,
        limit: Option<usize>,
    ) -> Result<Vec<(String, String)>> {
        let sql = match (limit, offset) {
            (Some(limit_value), _) => format!(
                "SELECT role, content FROM messages WHERE session_id = ?1 AND role NOT LIKE 'pending_user:%' ORDER BY id LIMIT {} OFFSET {}",
                limit_value, offset
            ),
            (None, 0) => {
                "SELECT role, content FROM messages WHERE session_id = ?1 AND role NOT LIKE 'pending_user:%' ORDER BY id".to_string()
            }
            (None, _) => format!(
                "SELECT role, content FROM messages WHERE session_id = ?1 AND role NOT LIKE 'pending_user:%' ORDER BY id LIMIT -1 OFFSET {}",
                offset
            ),
        };

        let mut stmt = self.db.conn().prepare(&sql)?;
        let messages = stmt.query_map([session_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        messages.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_message_count(&self, session_id: &str) -> Result<usize> {
        let count: i64 = self.db.conn().query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND role NOT LIKE 'pending_user:%'",
            [session_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    pub fn update_last_message(
        &self,
        session_id: &str,
        role: &str,
        content_json: &str,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let message = self
            .db
            .conn()
            .query_row(
                "SELECT id, created_at FROM messages
                 WHERE session_id = ?1 AND role = ?2
                 ORDER BY id DESC LIMIT 1",
                params![session_id, role],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let Some((message_id, created_at)) = message else {
            anyhow::bail!(
                "No {} message found to update in session {}",
                role,
                session_id
            );
        };
        self.db.conn().execute(
            "UPDATE messages SET content = ?1 WHERE id = ?2",
            params![content_json, message_id],
        )?;
        self.db.conn().execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;
        if let Err(error) = EpisodeStore::new(self.db).record_message(
            session_id,
            message_id,
            role,
            content_json,
            &created_at,
        ) {
            tracing::warn!(
                session_id,
                message_id,
                error = %error,
                "Edited canonical message saved but episodic recall indexing failed"
            );
        }
        Ok(())
    }

    pub fn delete_session_messages(&self, session_id: &str) -> Result<()> {
        self.db
            .conn()
            .execute("DELETE FROM messages WHERE session_id = ?1", [session_id])?;
        Ok(())
    }
}
