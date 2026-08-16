use std::io::{Error as IoError, ErrorKind};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction, TransactionBehavior};
use serde_json::Value;

use crate::storage::hive_workers::HiveWorker;
use crate::storage::Database;

use super::super::hive_workers::store::{map_worker, WORKER_COLUMNS};
use super::model::{
    HiveGroup, HiveGroupExecutionMode, HiveGroupMember, HiveGroupMessage, HiveGroupSenderKind,
    HiveGroupStatus, HiveGroupTurn, HiveGroupTurnPolicy, HiveGroupTurnStatus, HiveGroupUpdate,
    HiveMemberCursor, NewHiveGroup, NewHiveGroupMessage, MAX_HIVE_GROUP_MESSAGE_BYTES,
};

const GROUP_COLUMNS: &str = "id, user_id, title, execution_mode, max_rounds, max_member_messages_per_turn, parallelism, context_window_messages, status, default_assignee_worker_id, created_at, updated_at";
const MESSAGE_COLUMNS: &str = "id, group_id, seq, sender_kind, sender_worker_id, sender_run_id, content, reply_to_message_id, turn_id, idempotency_key, created_at";
const TURN_COLUMNS: &str = "id, group_id, trigger_message_id, execution_mode, policy_json, speaker_plan_json, next_speaker_index, status, member_outcomes_json, started_at, finished_at, created_at, updated_at";

/// Matches rows owned by exactly the given user (NULL = local), mirroring
/// the exact-owner semantics of the other hive stores.
const OWNER_PREDICATE: &str = "((?1 IS NULL AND user_id IS NULL) OR user_id = ?1)";

const MAX_GROUP_TITLE_BYTES: usize = 200;

/// Result of a cap-enforced worker message append.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CappedGroupAppend {
    /// The message was appended; `posted` counts this run's posts so far
    /// (including this one).
    Appended {
        message: Box<HiveGroupMessage>,
        posted: u32,
    },
    /// The run already posted `cap` messages in this turn.
    CapExceeded { cap: u32, posted: u32 },
}

pub struct HiveGroupStore {
    db: Database,
}

impl HiveGroupStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Create a group and its ordered membership atomically. Every member
    /// must be a non-archived Worker of the same owner.
    pub fn create(&self, input: &NewHiveGroup) -> Result<HiveGroup> {
        let title = normalized_group_title(&input.title)?;
        validate_caps(
            input.max_rounds.unwrap_or(3),
            input.max_member_messages_per_turn.unwrap_or(2),
            input.parallelism.unwrap_or(3),
            input.context_window_messages.unwrap_or(24),
        )?;
        anyhow::ensure!(
            !input.member_worker_ids.is_empty(),
            "a group needs at least one member Worker"
        );

        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        validate_member_workers(&tx, input.user_id.as_deref(), &input.member_worker_ids)?;
        if let Some(assignee) = input.default_assignee_worker_id.as_deref() {
            anyhow::ensure!(
                input
                    .member_worker_ids
                    .iter()
                    .any(|member| member == assignee),
                "the default assignee must be a group member"
            );
        }
        tx.execute(
            "INSERT INTO hive_groups (
                id, user_id, title, execution_mode, max_rounds,
                max_member_messages_per_turn, parallelism, context_window_messages,
                status, default_assignee_worker_id, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'active', ?9, ?10, ?10)",
            params![
                id,
                input.user_id,
                title,
                input.execution_mode.as_str(),
                input.max_rounds.unwrap_or(3),
                input.max_member_messages_per_turn.unwrap_or(2),
                input.parallelism.unwrap_or(3),
                input.context_window_messages.unwrap_or(24),
                input.default_assignee_worker_id,
                now,
            ],
        )
        .context("inserting Hive group")?;
        replace_members(&tx, &id, &input.member_worker_ids, &now)?;
        let group = load_group(&tx, &id)?
            .ok_or_else(|| anyhow::anyhow!("failed to load newly created Hive group {id}"))?;
        tx.commit()?;
        Ok(group)
    }

    pub fn get(&self, id: &str) -> Result<Option<HiveGroup>> {
        load_group(self.db.conn(), id)
    }

    pub fn list_for_owner(
        &self,
        user_id: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<HiveGroup>> {
        let status_predicate = if include_archived {
            ""
        } else {
            " AND status <> 'archived'"
        };
        let sql = format!(
            "SELECT {GROUP_COLUMNS} FROM hive_groups
             WHERE {OWNER_PREDICATE}{status_predicate}
             ORDER BY title ASC, created_at ASC"
        );
        let mut statement = self.db.conn().prepare(&sql)?;
        let groups = statement
            .query_map(params![user_id], map_group)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("listing Hive groups")?;
        Ok(groups)
    }

    /// Overwrite the editable policy surface of one group.
    pub fn update_settings(&self, id: &str, update: &HiveGroupUpdate) -> Result<Option<HiveGroup>> {
        let title = normalized_group_title(&update.title)?;
        validate_caps(
            update.max_rounds,
            update.max_member_messages_per_turn,
            update.parallelism,
            update.context_window_messages,
        )?;
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        if let Some(assignee) = update.default_assignee_worker_id.as_deref() {
            anyhow::ensure!(
                is_member(&tx, id, assignee)?,
                "the default assignee must be a group member"
            );
        }
        let changed = tx.execute(
            "UPDATE hive_groups
             SET title = ?2, execution_mode = ?3, max_rounds = ?4,
                 max_member_messages_per_turn = ?5, parallelism = ?6,
                 context_window_messages = ?7, default_assignee_worker_id = ?8,
                 updated_at = ?9
             WHERE id = ?1",
            params![
                id,
                title,
                update.execution_mode.as_str(),
                update.max_rounds,
                update.max_member_messages_per_turn,
                update.parallelism,
                update.context_window_messages,
                update.default_assignee_worker_id,
                chrono::Utc::now().to_rfc3339(),
            ],
        )?;
        if changed == 0 {
            tx.commit()?;
            return Ok(None);
        }
        let group = load_group(&tx, id)?;
        tx.commit()?;
        Ok(group)
    }

    pub fn set_status(&self, id: &str, status: HiveGroupStatus) -> Result<bool> {
        let changed = self.db.conn().execute(
            "UPDATE hive_groups SET status = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, status.as_str(), chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(changed == 1)
    }

    /// Replace the ordered membership. Retained members keep their original
    /// `added_at`; a removed default assignee is cleared, never dangling.
    pub fn set_members(&self, group_id: &str, ordered_worker_ids: &[String]) -> Result<()> {
        anyhow::ensure!(
            !ordered_worker_ids.is_empty(),
            "a group needs at least one member Worker"
        );
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let group = load_group(&tx, group_id)?
            .ok_or_else(|| anyhow::anyhow!("Hive group {group_id} not found"))?;
        validate_member_workers(&tx, group.user_id.as_deref(), ordered_worker_ids)?;
        let now = chrono::Utc::now().to_rfc3339();
        replace_members(&tx, group_id, ordered_worker_ids, &now)?;
        if let Some(assignee) = group.default_assignee_worker_id.as_deref() {
            if !ordered_worker_ids.iter().any(|member| member == assignee) {
                tx.execute(
                    "UPDATE hive_groups SET default_assignee_worker_id = NULL, updated_at = ?2
                     WHERE id = ?1",
                    params![group_id, now],
                )?;
            }
        }
        tx.execute(
            "UPDATE hive_groups SET updated_at = ?2 WHERE id = ?1",
            params![group_id, now],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn members(&self, group_id: &str) -> Result<Vec<HiveGroupMember>> {
        let mut statement = self.db.conn().prepare(
            "SELECT group_id, worker_id, position, added_at FROM hive_group_members
             WHERE group_id = ?1 ORDER BY position ASC",
        )?;
        let members = statement
            .query_map([group_id], map_member)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("listing Hive group members")?;
        Ok(members)
    }

    /// Member Workers in roster order.
    pub fn member_workers(&self, group_id: &str) -> Result<Vec<HiveWorker>> {
        load_member_workers(self.db.conn(), group_id)
    }

    /// Append one room message with transactional sequence allocation. An
    /// existing `(group, idempotency_key)` row is returned unchanged.
    pub fn append_message(&self, input: &NewHiveGroupMessage) -> Result<HiveGroupMessage> {
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let now = chrono::Utc::now().to_rfc3339();
        let message = append_message_with_conn(&tx, input, &now)?;
        tx.commit()?;
        Ok(message)
    }

    /// Append a worker message only while the posting run stays under the
    /// turn's per-run message cap. Count and insert share one transaction so
    /// parallel posts cannot slip past the cap.
    pub fn append_worker_message_capped(
        &self,
        input: &NewHiveGroupMessage,
        cap: u32,
    ) -> Result<CappedGroupAppend> {
        anyhow::ensure!(
            input.sender_kind == HiveGroupSenderKind::Worker,
            "capped appends are for worker messages"
        );
        let turn_id = input
            .turn_id
            .as_deref()
            .context("capped appends require a turn id")?;
        let run_id = input
            .sender_run_id
            .as_deref()
            .context("capped appends require the posting run id")?;
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let posted: u32 = tx.query_row(
            "SELECT COUNT(*) FROM hive_group_messages
             WHERE turn_id = ?1 AND sender_run_id = ?2",
            params![turn_id, run_id],
            |row| row.get(0),
        )?;
        if posted >= cap {
            tx.commit()?;
            return Ok(CappedGroupAppend::CapExceeded { cap, posted });
        }
        let now = chrono::Utc::now().to_rfc3339();
        let message = append_message_with_conn(&tx, input, &now)?;
        tx.commit()?;
        Ok(CappedGroupAppend::Appended {
            message: Box::new(message),
            posted: posted + 1,
        })
    }

    pub fn get_message(&self, id: &str) -> Result<Option<HiveGroupMessage>> {
        let sql = format!("SELECT {MESSAGE_COLUMNS} FROM hive_group_messages WHERE id = ?1");
        self.db
            .conn()
            .query_row(&sql, [id], map_message)
            .optional()
            .context("reading Hive group message")
    }

    /// Messages after a sequence cursor in ascending order.
    pub fn list_messages_after(
        &self,
        group_id: &str,
        after_seq: i64,
        limit: usize,
    ) -> Result<Vec<HiveGroupMessage>> {
        let sql = format!(
            "SELECT {MESSAGE_COLUMNS} FROM hive_group_messages
             WHERE group_id = ?1 AND seq > ?2
             ORDER BY seq ASC LIMIT ?3"
        );
        let mut statement = self.db.conn().prepare(&sql)?;
        let messages = statement
            .query_map(params![group_id, after_seq, limit as i64], map_message)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("listing Hive group messages")?;
        Ok(messages)
    }

    /// The last `limit` messages in ascending order.
    pub fn list_recent_messages(
        &self,
        group_id: &str,
        limit: usize,
    ) -> Result<Vec<HiveGroupMessage>> {
        load_recent_messages(self.db.conn(), group_id, limit)
    }

    pub fn latest_seq(&self, group_id: &str) -> Result<i64> {
        latest_seq_with_conn(self.db.conn(), group_id)
    }

    pub fn get_turn(&self, id: &str) -> Result<Option<HiveGroupTurn>> {
        load_turn(self.db.conn(), id)
    }

    /// The most recent turns for a group, newest first.
    pub fn list_turns(&self, group_id: &str, limit: usize) -> Result<Vec<HiveGroupTurn>> {
        let sql = format!(
            "SELECT {TURN_COLUMNS} FROM hive_group_turns
             WHERE group_id = ?1 ORDER BY started_at DESC, created_at DESC LIMIT ?2"
        );
        let mut statement = self.db.conn().prepare(&sql)?;
        let turns = statement
            .query_map(params![group_id, limit as i64], map_turn)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("listing Hive group turns")?;
        Ok(turns)
    }

    pub fn active_turn(&self, group_id: &str) -> Result<Option<HiveGroupTurn>> {
        load_active_turn(self.db.conn(), group_id)
    }

    pub fn cursor(&self, group_id: &str, worker_id: &str) -> Result<Option<HiveMemberCursor>> {
        self.db
            .conn()
            .query_row(
                "SELECT group_id, worker_id, last_seen_seq, last_spoke_seq, updated_at
                 FROM hive_member_cursors WHERE group_id = ?1 AND worker_id = ?2",
                params![group_id, worker_id],
                map_cursor,
            )
            .optional()
            .context("reading Hive member cursor")
    }

    pub fn advance_cursor(
        &self,
        group_id: &str,
        worker_id: &str,
        seen_seq: Option<i64>,
        spoke_seq: Option<i64>,
    ) -> Result<()> {
        advance_member_cursor_with_conn(
            self.db.conn(),
            group_id,
            worker_id,
            seen_seq,
            spoke_seq,
            &chrono::Utc::now().to_rfc3339(),
        )
    }

    #[cfg(test)]
    pub(super) fn conn(&self) -> &rusqlite::Connection {
        self.db.conn()
    }
}

/// Load one group by id over any connection (daemon transactions included).
pub fn load_group(conn: &Connection, id: &str) -> Result<Option<HiveGroup>> {
    let sql = format!("SELECT {GROUP_COLUMNS} FROM hive_groups WHERE id = ?1");
    conn.query_row(&sql, [id], map_group)
        .optional()
        .context("reading Hive group")
}

/// Member Workers in roster order over any connection.
pub fn load_member_workers(conn: &Connection, group_id: &str) -> Result<Vec<HiveWorker>> {
    let sql = format!(
        "SELECT {WORKER_COLUMNS} FROM hive_group_members m
         JOIN hive_workers w ON w.id = m.worker_id
         WHERE m.group_id = ?1
         ORDER BY m.position ASC"
    );
    let mut statement = conn.prepare(&sql)?;
    let workers = statement
        .query_map([group_id], map_worker)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("listing Hive group member workers")?;
    Ok(workers)
}

/// Append one message with per-group monotonic sequence allocation inside the
/// caller's transaction. Idempotent on `(group_id, idempotency_key)`.
pub fn append_message_with_conn(
    conn: &Connection,
    input: &NewHiveGroupMessage,
    now: &str,
) -> Result<HiveGroupMessage> {
    let content = input.content.trim();
    anyhow::ensure!(!content.is_empty(), "group message must not be empty");
    anyhow::ensure!(
        content.len() <= MAX_HIVE_GROUP_MESSAGE_BYTES,
        "group message exceeds {MAX_HIVE_GROUP_MESSAGE_BYTES} bytes"
    );
    match input.sender_kind {
        HiveGroupSenderKind::Worker => anyhow::ensure!(
            input.sender_worker_id.is_some(),
            "worker messages require a sender worker"
        ),
        HiveGroupSenderKind::User | HiveGroupSenderKind::System => anyhow::ensure!(
            input.sender_worker_id.is_none() && input.sender_run_id.is_none(),
            "only worker messages carry a sender worker or run"
        ),
    }

    if let Some(key) = input.idempotency_key.as_deref() {
        let sql = format!(
            "SELECT {MESSAGE_COLUMNS} FROM hive_group_messages
             WHERE group_id = ?1 AND idempotency_key = ?2"
        );
        if let Some(existing) = conn
            .query_row(&sql, params![input.group_id, key], map_message)
            .optional()?
        {
            return Ok(existing);
        }
    }
    if let Some(reply_to) = input.reply_to_message_id.as_deref() {
        let reply_group: Option<String> = conn
            .query_row(
                "SELECT group_id FROM hive_group_messages WHERE id = ?1",
                [reply_to],
                |row| row.get(0),
            )
            .optional()?;
        anyhow::ensure!(
            reply_group.as_deref() == Some(input.group_id.as_str()),
            "reply target is not a message in this group"
        );
    }

    let seq: i64 = conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM hive_group_messages WHERE group_id = ?1",
        [&input.group_id],
        |row| row.get(0),
    )?;
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO hive_group_messages (
            id, group_id, seq, sender_kind, sender_worker_id, sender_run_id,
            content, reply_to_message_id, turn_id, idempotency_key, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            id,
            input.group_id,
            seq,
            input.sender_kind.as_str(),
            input.sender_worker_id,
            input.sender_run_id,
            content,
            input.reply_to_message_id,
            input.turn_id,
            input.idempotency_key,
            now,
        ],
    )
    .context("appending Hive group message")?;
    if let Some(worker_id) = input.sender_worker_id.as_deref() {
        advance_member_cursor_with_conn(
            conn,
            &input.group_id,
            worker_id,
            Some(seq),
            Some(seq),
            now,
        )?;
    }
    Ok(HiveGroupMessage {
        id,
        group_id: input.group_id.clone(),
        seq,
        sender_kind: input.sender_kind,
        sender_worker_id: input.sender_worker_id.clone(),
        sender_run_id: input.sender_run_id.clone(),
        content: content.to_string(),
        reply_to_message_id: input.reply_to_message_id.clone(),
        turn_id: input.turn_id.clone(),
        idempotency_key: input.idempotency_key.clone(),
        created_at: now.to_string(),
    })
}

/// The last `limit` messages of a group in ascending order.
pub fn load_recent_messages(
    conn: &Connection,
    group_id: &str,
    limit: usize,
) -> Result<Vec<HiveGroupMessage>> {
    let sql = format!(
        "SELECT {MESSAGE_COLUMNS} FROM (
             SELECT {MESSAGE_COLUMNS} FROM hive_group_messages
             WHERE group_id = ?1 ORDER BY seq DESC LIMIT ?2
         ) ORDER BY seq ASC"
    );
    let mut statement = conn.prepare(&sql)?;
    let messages = statement
        .query_map(params![group_id, limit as i64], map_message)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("listing recent Hive group messages")?;
    Ok(messages)
}

pub fn latest_seq_with_conn(conn: &Connection, group_id: &str) -> Result<i64> {
    conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) FROM hive_group_messages WHERE group_id = ?1",
        [group_id],
        |row| row.get(0),
    )
    .context("reading Hive group high-water sequence")
}

/// Insert a fully formed turn row inside the caller's transaction.
pub fn insert_turn_with_conn(conn: &Connection, turn: &HiveGroupTurn) -> Result<()> {
    conn.execute(
        "INSERT INTO hive_group_turns (
            id, group_id, trigger_message_id, execution_mode, policy_json,
            speaker_plan_json, next_speaker_index, status, member_outcomes_json,
            started_at, finished_at, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            turn.id,
            turn.group_id,
            turn.trigger_message_id,
            turn.execution_mode.as_str(),
            serde_json::to_string(&turn.policy).context("encoding turn policy")?,
            serde_json::to_string(&turn.speaker_plan).context("encoding speaker plan")?,
            turn.next_speaker_index,
            turn.status.as_str(),
            turn.member_outcomes
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .context("encoding member outcomes")?,
            turn.started_at,
            turn.finished_at,
            turn.created_at,
            turn.updated_at,
        ],
    )
    .context("inserting Hive group turn")?;
    Ok(())
}

pub fn load_turn(conn: &Connection, id: &str) -> Result<Option<HiveGroupTurn>> {
    let sql = format!("SELECT {TURN_COLUMNS} FROM hive_group_turns WHERE id = ?1");
    conn.query_row(&sql, [id], map_turn)
        .optional()
        .context("reading Hive group turn")
}

pub fn load_active_turn(conn: &Connection, group_id: &str) -> Result<Option<HiveGroupTurn>> {
    let sql = format!(
        "SELECT {TURN_COLUMNS} FROM hive_group_turns
         WHERE group_id = ?1 AND status = 'running'
         ORDER BY started_at DESC, created_at DESC LIMIT 1"
    );
    conn.query_row(&sql, [group_id], map_turn)
        .optional()
        .context("reading active Hive group turn")
}

/// Advance the roundtable speaker cursor of a running turn.
pub fn update_turn_progress_with_conn(
    conn: &Connection,
    id: &str,
    next_speaker_index: u32,
    now: &str,
) -> Result<bool> {
    let changed = conn.execute(
        "UPDATE hive_group_turns
         SET next_speaker_index = ?2, updated_at = ?3
         WHERE id = ?1 AND status = 'running'",
        params![id, next_speaker_index, now],
    )?;
    Ok(changed == 1)
}

/// Record the latest per-member outcome summaries without finishing the turn.
pub fn update_turn_member_outcomes_with_conn(
    conn: &Connection,
    id: &str,
    member_outcomes: &Value,
    now: &str,
) -> Result<bool> {
    let changed = conn.execute(
        "UPDATE hive_group_turns
         SET member_outcomes_json = ?2, updated_at = ?3
         WHERE id = ?1 AND status = 'running'",
        params![
            id,
            serde_json::to_string(member_outcomes).context("encoding member outcomes")?,
            now
        ],
    )?;
    Ok(changed == 1)
}

/// Move a running turn to a terminal status exactly once.
pub fn finalize_turn_with_conn(
    conn: &Connection,
    id: &str,
    status: HiveGroupTurnStatus,
    member_outcomes: Option<&Value>,
    now: &str,
) -> Result<bool> {
    anyhow::ensure!(
        status.is_terminal(),
        "turn finalization requires a terminal status"
    );
    let changed = conn.execute(
        "UPDATE hive_group_turns
         SET status = ?2,
             member_outcomes_json = COALESCE(?3, member_outcomes_json),
             finished_at = ?4, updated_at = ?4
         WHERE id = ?1 AND status = 'running'",
        params![
            id,
            status.as_str(),
            member_outcomes
                .map(serde_json::to_string)
                .transpose()
                .context("encoding member outcomes")?,
            now
        ],
    )?;
    Ok(changed == 1)
}

/// Monotonic cursor upsert; sequences only move forward.
pub fn advance_member_cursor_with_conn(
    conn: &Connection,
    group_id: &str,
    worker_id: &str,
    seen_seq: Option<i64>,
    spoke_seq: Option<i64>,
    now: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO hive_member_cursors (
            group_id, worker_id, last_seen_seq, last_spoke_seq, updated_at
         ) VALUES (?1, ?2, COALESCE(?3, 0), COALESCE(?4, 0), ?5)
         ON CONFLICT(group_id, worker_id) DO UPDATE SET
             last_seen_seq = MAX(last_seen_seq, COALESCE(?3, 0)),
             last_spoke_seq = MAX(last_spoke_seq, COALESCE(?4, 0)),
             updated_at = ?5",
        params![group_id, worker_id, seen_seq, spoke_seq, now],
    )
    .context("advancing Hive member cursor")?;
    Ok(())
}

fn normalized_group_title(title: &str) -> Result<String> {
    let title = title.trim();
    anyhow::ensure!(!title.is_empty(), "group title must not be empty");
    anyhow::ensure!(
        title.len() <= MAX_GROUP_TITLE_BYTES,
        "group title exceeds {MAX_GROUP_TITLE_BYTES} bytes"
    );
    Ok(title.to_string())
}

fn validate_caps(
    max_rounds: u32,
    max_member_messages_per_turn: u32,
    parallelism: u32,
    context_window_messages: u32,
) -> Result<()> {
    for (label, value) in [
        ("max_rounds", max_rounds),
        ("max_member_messages_per_turn", max_member_messages_per_turn),
        ("parallelism", parallelism),
        ("context_window_messages", context_window_messages),
    ] {
        anyhow::ensure!(value > 0, "{label} must be positive");
        anyhow::ensure!(value <= 1000, "{label} exceeds the sane limit of 1000");
    }
    Ok(())
}

/// Every member must exist, belong to the same owner, and not be archived.
fn validate_member_workers(
    conn: &Connection,
    user_id: Option<&str>,
    worker_ids: &[String],
) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    for worker_id in worker_ids {
        anyhow::ensure!(
            seen.insert(worker_id.as_str()),
            "duplicate group member {worker_id}"
        );
        let row: Option<(Option<String>, String)> = conn
            .query_row(
                "SELECT user_id, status FROM hive_workers WHERE id = ?1",
                [worker_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((owner, status)) = row else {
            anyhow::bail!("group member Worker {worker_id} not found");
        };
        anyhow::ensure!(
            owner.as_deref() == user_id,
            "group member Worker {worker_id} not found"
        );
        anyhow::ensure!(
            status != "archived",
            "group member Worker {worker_id} is archived"
        );
    }
    Ok(())
}

fn replace_members(
    conn: &Connection,
    group_id: &str,
    ordered_worker_ids: &[String],
    now: &str,
) -> Result<()> {
    let placeholders = ordered_worker_ids
        .iter()
        .enumerate()
        .map(|(index, _)| format!("?{}", index + 2))
        .collect::<Vec<_>>()
        .join(", ");
    let delete_sql = format!(
        "DELETE FROM hive_group_members
         WHERE group_id = ?1 AND worker_id NOT IN ({placeholders})"
    );
    let mut delete_params: Vec<&dyn rusqlite::ToSql> = vec![&group_id];
    for worker_id in ordered_worker_ids {
        delete_params.push(worker_id);
    }
    conn.execute(&delete_sql, delete_params.as_slice())?;
    for (position, worker_id) in ordered_worker_ids.iter().enumerate() {
        conn.execute(
            "INSERT INTO hive_group_members (group_id, worker_id, position, added_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(group_id, worker_id) DO UPDATE SET position = excluded.position",
            params![group_id, worker_id, position as i64, now],
        )?;
    }
    Ok(())
}

fn is_member(conn: &Connection, group_id: &str, worker_id: &str) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM hive_group_members WHERE group_id = ?1 AND worker_id = ?2
         )",
        params![group_id, worker_id],
        |row| row.get(0),
    )
    .context("checking Hive group membership")
}

fn map_group(row: &Row<'_>) -> rusqlite::Result<HiveGroup> {
    let execution_mode =
        parse_required(3, row.get::<_, String>(3)?, HiveGroupExecutionMode::parse)?;
    let status = parse_required(8, row.get::<_, String>(8)?, HiveGroupStatus::parse)?;
    Ok(HiveGroup {
        id: row.get(0)?,
        user_id: row.get(1)?,
        title: row.get(2)?,
        execution_mode,
        max_rounds: positive_u32(row, 4)?,
        max_member_messages_per_turn: positive_u32(row, 5)?,
        parallelism: positive_u32(row, 6)?,
        context_window_messages: positive_u32(row, 7)?,
        status,
        default_assignee_worker_id: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

fn map_member(row: &Row<'_>) -> rusqlite::Result<HiveGroupMember> {
    Ok(HiveGroupMember {
        group_id: row.get(0)?,
        worker_id: row.get(1)?,
        position: positive_or_zero_u32(row, 2)?,
        added_at: row.get(3)?,
    })
}

fn map_message(row: &Row<'_>) -> rusqlite::Result<HiveGroupMessage> {
    let sender_kind = parse_required(3, row.get::<_, String>(3)?, HiveGroupSenderKind::parse)?;
    Ok(HiveGroupMessage {
        id: row.get(0)?,
        group_id: row.get(1)?,
        seq: row.get(2)?,
        sender_kind,
        sender_worker_id: row.get(4)?,
        sender_run_id: row.get(5)?,
        content: row.get(6)?,
        reply_to_message_id: row.get(7)?,
        turn_id: row.get(8)?,
        idempotency_key: row.get(9)?,
        created_at: row.get(10)?,
    })
}

fn map_turn(row: &Row<'_>) -> rusqlite::Result<HiveGroupTurn> {
    let execution_mode =
        parse_required(3, row.get::<_, String>(3)?, HiveGroupExecutionMode::parse)?;
    let status = parse_required(7, row.get::<_, String>(7)?, HiveGroupTurnStatus::parse)?;
    let policy = serde_json::from_str::<HiveGroupTurnPolicy>(&row.get::<_, String>(4)?)
        .map_err(|error| conversion_error(4, format!("invalid turn policy JSON: {error}")))?;
    let speaker_plan = serde_json::from_str::<Vec<String>>(&row.get::<_, String>(5)?)
        .map_err(|error| conversion_error(5, format!("invalid speaker plan JSON: {error}")))?;
    let member_outcomes = row
        .get::<_, Option<String>>(8)?
        .map(|value| {
            serde_json::from_str::<Value>(&value).map_err(|error| {
                conversion_error(8, format!("invalid member outcomes JSON: {error}"))
            })
        })
        .transpose()?;
    Ok(HiveGroupTurn {
        id: row.get(0)?,
        group_id: row.get(1)?,
        trigger_message_id: row.get(2)?,
        execution_mode,
        policy,
        speaker_plan,
        next_speaker_index: positive_or_zero_u32(row, 6)?,
        status,
        member_outcomes,
        started_at: row.get(9)?,
        finished_at: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

fn map_cursor(row: &Row<'_>) -> rusqlite::Result<HiveMemberCursor> {
    Ok(HiveMemberCursor {
        group_id: row.get(0)?,
        worker_id: row.get(1)?,
        last_seen_seq: row.get(2)?,
        last_spoke_seq: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

fn positive_u32(row: &Row<'_>, index: usize) -> rusqlite::Result<u32> {
    let value = row.get::<_, i64>(index)?;
    u32::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| conversion_error(index, "value must be a positive u32"))
}

fn positive_or_zero_u32(row: &Row<'_>, index: usize) -> rusqlite::Result<u32> {
    let value = row.get::<_, i64>(index)?;
    u32::try_from(value).map_err(|_| conversion_error(index, "value must be a non-negative u32"))
}

fn parse_required<T>(
    index: usize,
    value: String,
    parse: impl FnOnce(&str) -> Option<T>,
) -> rusqlite::Result<T> {
    parse(&value).ok_or_else(|| conversion_error(index, format!("invalid enum value: {value}")))
}

fn conversion_error(index: usize, message: impl std::fmt::Display) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        index,
        rusqlite::types::Type::Text,
        Box::new(IoError::new(ErrorKind::InvalidData, message.to_string())),
    )
}
