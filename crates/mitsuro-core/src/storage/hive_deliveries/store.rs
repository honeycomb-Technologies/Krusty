use std::io::{Error as IoError, ErrorKind};

use anyhow::{Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::hive::canonical_timestamp;
use crate::storage::Database;

use super::model::{
    HiveDelivery, HiveDeliveryEnqueue, HiveDeliveryKind, HiveDeliveryPriority, HiveDeliveryStatus,
    NewHiveDelivery, MAX_HIVE_DELIVERY_BODY_BYTES,
};

pub(crate) const DELIVERY_COLUMNS: &str = "id, kind, from_worker_id, to_worker_id, group_id, body, priority, dedupe_key, status, attempt_count, max_attempts, available_at, delivered_at, acked_at, last_error, run_id, created_at, updated_at";

/// Run statuses that still hold (or will hold) the recipient's DM lane.
const LANE_BUSY_RUN_STATUSES: &str =
    "('queued', 'leased', 'running', 'sleeping', 'retry_wait', 'awaiting_input', 'recovery_required')";

/// Run statuses that acknowledge a woken run's delivery as consumed.
const ACK_TERMINAL_RUN_STATUSES: &str = "('succeeded', 'failed', 'cancelled', 'dead_letter')";

const DEAD_LETTER_EXHAUSTED: &str = "delivery attempts exhausted";
const DEAD_LETTER_RECIPIENT_ARCHIVED: &str = "the recipient Worker is archived";

/// Exponential redelivery backoff. The claim itself schedules the next
/// attempt at this offset, so a daemon crash between claim and effect
/// replays the delivery after the backoff instead of losing it.
pub fn hive_delivery_retry_backoff(attempt: u32) -> ChronoDuration {
    const BASE_SECS: i64 = 5;
    const MAX_SECS: i64 = 300;
    let exponent = attempt.saturating_sub(1).min(16);
    ChronoDuration::seconds((BASE_SECS << exponent).min(MAX_SECS))
}

pub struct HiveDeliveryStore {
    db: Database,
}

impl HiveDeliveryStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Idempotently enqueue one delivery. A retried enqueue carrying the same
    /// dedupe key adopts the existing row instead of duplicating it.
    pub fn enqueue(&self, input: &NewHiveDelivery) -> Result<HiveDeliveryEnqueue> {
        enqueue_with_conn(self.db.conn(), input, Utc::now())
    }

    pub fn get(&self, id: &str) -> Result<Option<HiveDelivery>> {
        load_delivery(self.db.conn(), id)
    }

    /// Ledger rows touching one Worker in either direction, newest first,
    /// optionally filtered to one status.
    pub fn list_for_worker(
        &self,
        worker_id: &str,
        status: Option<HiveDeliveryStatus>,
        limit: usize,
    ) -> Result<Vec<HiveDelivery>> {
        let status_predicate = if status.is_some() {
            " AND status = ?2"
        } else {
            ""
        };
        let sql = format!(
            "SELECT {DELIVERY_COLUMNS} FROM hive_deliveries
             WHERE (to_worker_id = ?1 OR from_worker_id = ?1){status_predicate}
             ORDER BY created_at DESC, id DESC
             LIMIT {}",
            limit.clamp(1, 500)
        );
        let mut statement = self.db.conn().prepare(&sql)?;
        let rows = match status {
            Some(status) => statement
                .query_map(params![worker_id, status.as_str()], map_delivery)?
                .collect::<rusqlite::Result<Vec<_>>>(),
            None => statement
                .query_map([worker_id], map_delivery)?
                .collect::<rusqlite::Result<Vec<_>>>(),
        }
        .context("listing Hive deliveries for worker")?;
        Ok(rows)
    }
}

pub fn enqueue_with_conn(
    conn: &Connection,
    input: &NewHiveDelivery,
    now: DateTime<Utc>,
) -> Result<HiveDeliveryEnqueue> {
    let body = input.body.trim();
    anyhow::ensure!(!body.is_empty(), "delivery body must not be empty");
    anyhow::ensure!(
        body.len() <= MAX_HIVE_DELIVERY_BODY_BYTES,
        "delivery body exceeds {MAX_HIVE_DELIVERY_BODY_BYTES} bytes"
    );
    anyhow::ensure!(input.max_attempts > 0, "max_attempts must be positive");
    let dedupe_key = input
        .dedupe_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    anyhow::ensure!(
        dedupe_key.is_none_or(|value| value.len() <= 512),
        "dedupe key exceeds 512 bytes"
    );

    if let Some(dedupe_key) = dedupe_key {
        if let Some(existing) = conn
            .query_row(
                &format!("SELECT {DELIVERY_COLUMNS} FROM hive_deliveries WHERE dedupe_key = ?1"),
                [dedupe_key],
                map_delivery,
            )
            .optional()
            .context("checking Hive delivery dedupe key")?
        {
            return Ok(HiveDeliveryEnqueue {
                delivery: existing,
                deduplicated: true,
            });
        }
    }

    let id = uuid::Uuid::new_v4().to_string();
    let now_text = canonical_timestamp(now);
    conn.execute(
        "INSERT INTO hive_deliveries (
            id, kind, from_worker_id, to_worker_id, group_id, body, priority,
            dedupe_key, status, attempt_count, max_attempts, available_at,
            delivered_at, acked_at, last_error, run_id, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', 0, ?9, ?10,
                   NULL, NULL, NULL, NULL, ?10, ?10)
         ON CONFLICT(dedupe_key) DO NOTHING",
        params![
            id,
            input.kind.as_str(),
            input.from_worker_id,
            input.to_worker_id,
            input.group_id,
            body,
            input.priority.as_str(),
            dedupe_key,
            input.max_attempts,
            now_text,
        ],
    )
    .context("inserting Hive delivery")?;

    // A concurrent enqueue with the same dedupe key may have won the insert
    // race; the unique row that exists now is authoritative either way.
    if let Some(dedupe_key) = dedupe_key {
        let existing = conn
            .query_row(
                &format!("SELECT {DELIVERY_COLUMNS} FROM hive_deliveries WHERE dedupe_key = ?1"),
                [dedupe_key],
                map_delivery,
            )
            .context("reading enqueued Hive delivery")?;
        let deduplicated = existing.id != id;
        return Ok(HiveDeliveryEnqueue {
            delivery: existing,
            deduplicated,
        });
    }
    let delivery = load_delivery(conn, &id)?
        .ok_or_else(|| anyhow::anyhow!("failed to load newly enqueued Hive delivery {id}"))?;
    Ok(HiveDeliveryEnqueue {
        delivery,
        deduplicated: false,
    })
}

pub fn load_delivery(conn: &Connection, id: &str) -> Result<Option<HiveDelivery>> {
    conn.query_row(
        &format!("SELECT {DELIVERY_COLUMNS} FROM hive_deliveries WHERE id = ?1"),
        [id],
        map_delivery,
    )
    .optional()
    .context("reading Hive delivery")
}

/// Claim the due deliveries the pump can act on right now.
///
/// Within the same transaction, first dead-letter rows that can never
/// deliver (archived recipient) or that exhausted their attempt budget,
/// then move claimable rows to `delivering` with one more attempt and a
/// redelivery backoff on `available_at`. A `high` delivery claims even when
/// the recipient's lane is busy (it will steer); a `normal` delivery is left
/// pending untouched until the lane is idle, so waiting never burns
/// attempts. Rows still in `delivering` are crash leftovers and are
/// reclaimed once their backoff elapses.
pub fn claim_due_with_conn(
    conn: &Connection,
    now: DateTime<Utc>,
    limit: usize,
) -> Result<Vec<HiveDelivery>> {
    let now_text = canonical_timestamp(now);
    conn.execute(
        "UPDATE hive_deliveries
         SET status = 'dead_letter', last_error = ?2, updated_at = ?1
         WHERE status IN ('pending', 'delivering') AND available_at <= ?1
           AND EXISTS (
               SELECT 1 FROM hive_workers w
               WHERE w.id = hive_deliveries.to_worker_id AND w.status = 'archived'
           )",
        params![now_text, DEAD_LETTER_RECIPIENT_ARCHIVED],
    )
    .context("dead-lettering Hive deliveries to archived workers")?;
    conn.execute(
        "UPDATE hive_deliveries
         SET status = 'dead_letter',
             last_error = COALESCE(last_error, ?2), updated_at = ?1
         WHERE status IN ('pending', 'delivering') AND available_at <= ?1
           AND attempt_count >= max_attempts",
        params![now_text, DEAD_LETTER_EXHAUSTED],
    )
    .context("dead-lettering exhausted Hive deliveries")?;

    let claimable = {
        let sql = format!(
            "SELECT d.id, d.attempt_count FROM hive_deliveries d
             JOIN hive_workers w ON w.id = d.to_worker_id
             WHERE d.status IN ('pending', 'delivering') AND d.available_at <= ?1
               AND d.attempt_count < d.max_attempts
               AND w.status = 'active'
               AND (
                   d.priority = 'high'
                   OR w.dm_session_id IS NULL
                   OR NOT EXISTS (
                       SELECT 1 FROM hive_runs r
                       JOIN hive_controllers c ON c.id = r.controller_id
                       WHERE c.session_id = w.dm_session_id
                         AND r.status IN {LANE_BUSY_RUN_STATUSES}
                   )
               )
             ORDER BY (d.priority = 'high') DESC, d.available_at, d.created_at, d.id
             LIMIT {}",
            limit.clamp(1, 100)
        );
        let mut statement = conn.prepare(&sql)?;
        let rows = statement
            .query_map([&now_text], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("selecting claimable Hive deliveries")?;
        rows
    };

    let mut claimed = Vec::with_capacity(claimable.len());
    for (id, attempt_count) in claimable {
        let retry_at = canonical_timestamp(now + hive_delivery_retry_backoff(attempt_count + 1));
        let changed = conn
            .execute(
                "UPDATE hive_deliveries
                 SET status = 'delivering', attempt_count = attempt_count + 1,
                     available_at = ?2, updated_at = ?3
                 WHERE id = ?1 AND status IN ('pending', 'delivering')",
                params![id, retry_at, now_text],
            )
            .context("claiming Hive delivery")?;
        if changed == 1 {
            if let Some(delivery) = load_delivery(conn, &id)? {
                claimed.push(delivery);
            }
        }
    }
    Ok(claimed)
}

/// Commit the durable effect of one claimed delivery. `ack` marks steered
/// deliveries consumed immediately; woken deliveries ack later when the
/// backreferenced run reaches a terminal state.
pub fn mark_delivered_with_conn(
    conn: &Connection,
    id: &str,
    run_id: Option<&str>,
    ack: bool,
    now: DateTime<Utc>,
) -> Result<bool> {
    let now_text = canonical_timestamp(now);
    let (status, acked_at) = if ack {
        ("acked", Some(now_text.clone()))
    } else {
        ("delivered", None)
    };
    let changed = conn
        .execute(
            "UPDATE hive_deliveries
             SET status = ?2, delivered_at = ?3, acked_at = ?4, last_error = NULL,
                 run_id = ?5, updated_at = ?3
             WHERE id = ?1 AND status = 'delivering'",
            params![id, status, now_text, acked_at, run_id],
        )
        .context("marking Hive delivery delivered")?;
    Ok(changed == 1)
}

/// Return a claimed delivery to `pending` without consuming an attempt.
/// Used when the recipient's lane state changed between claim and effect
/// (busy lane for a normal delivery, paused recipient): waiting is not a
/// failed delivery attempt.
pub fn revert_wait_with_conn(
    conn: &Connection,
    id: &str,
    retry_delay: ChronoDuration,
    now: DateTime<Utc>,
) -> Result<bool> {
    let now_text = canonical_timestamp(now);
    let retry_at = canonical_timestamp(now + retry_delay);
    let changed = conn
        .execute(
            "UPDATE hive_deliveries
             SET status = 'pending',
                 attempt_count = CASE WHEN attempt_count > 0 THEN attempt_count - 1 ELSE 0 END,
                 available_at = ?2, updated_at = ?3
             WHERE id = ?1 AND status = 'delivering'",
            params![id, retry_at, now_text],
        )
        .context("reverting Hive delivery to pending")?;
    Ok(changed == 1)
}

/// Record a failed delivery attempt: retry after backoff while attempts
/// remain, otherwise dead-letter with the error preserved.
pub fn fail_attempt_with_conn(
    conn: &Connection,
    id: &str,
    error: &str,
    now: DateTime<Utc>,
) -> Result<HiveDeliveryStatus> {
    let now_text = canonical_timestamp(now);
    let attempt_count: u32 = conn
        .query_row(
            "SELECT attempt_count FROM hive_deliveries WHERE id = ?1 AND status = 'delivering'",
            [id],
            |row| row.get(0),
        )
        .optional()
        .context("reading Hive delivery attempt count")?
        .unwrap_or(0);
    let retry_at = canonical_timestamp(now + hive_delivery_retry_backoff(attempt_count.max(1)));
    conn.execute(
        "UPDATE hive_deliveries
         SET status = CASE WHEN attempt_count >= max_attempts
                           THEN 'dead_letter' ELSE 'pending' END,
             available_at = ?2, last_error = ?3, updated_at = ?4
         WHERE id = ?1 AND status = 'delivering'",
        params![id, retry_at, error, now_text],
    )
    .context("recording failed Hive delivery attempt")?;
    let status = conn
        .query_row(
            "SELECT status FROM hive_deliveries WHERE id = ?1",
            [id],
            |row| row.get::<_, String>(0),
        )
        .context("reading Hive delivery status after failure")?;
    HiveDeliveryStatus::parse(&status)
        .ok_or_else(|| anyhow::anyhow!("invalid Hive delivery status: {status}"))
}

/// Acknowledge delivered rows whose woken run reached a terminal state.
/// Returns the acknowledged delivery ids.
pub fn ack_for_terminal_runs_with_conn(
    conn: &Connection,
    now: DateTime<Utc>,
) -> Result<Vec<String>> {
    let now_text = canonical_timestamp(now);
    let ids = {
        let sql = format!(
            "SELECT d.id FROM hive_deliveries d
             JOIN hive_runs r ON r.id = d.run_id
             WHERE d.status = 'delivered' AND r.status IN {ACK_TERMINAL_RUN_STATUSES}"
        );
        let mut statement = conn.prepare(&sql)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("selecting acknowledgeable Hive deliveries")?;
        rows
    };
    for id in &ids {
        conn.execute(
            "UPDATE hive_deliveries SET status = 'acked', acked_at = ?2, updated_at = ?2
             WHERE id = ?1 AND status = 'delivered'",
            params![id, now_text],
        )
        .context("acknowledging Hive delivery")?;
    }
    Ok(ids)
}

pub(crate) fn map_delivery(row: &Row<'_>) -> rusqlite::Result<HiveDelivery> {
    let kind = parse_required(1, row.get::<_, String>(1)?, HiveDeliveryKind::parse)?;
    let priority = parse_required(6, row.get::<_, String>(6)?, HiveDeliveryPriority::parse)?;
    let status = parse_required(8, row.get::<_, String>(8)?, HiveDeliveryStatus::parse)?;
    Ok(HiveDelivery {
        id: row.get(0)?,
        kind,
        from_worker_id: row.get(2)?,
        to_worker_id: row.get(3)?,
        group_id: row.get(4)?,
        body: row.get(5)?,
        priority,
        dedupe_key: row.get(7)?,
        status,
        attempt_count: row.get(9)?,
        max_attempts: row.get(10)?,
        available_at: row.get(11)?,
        delivered_at: row.get(12)?,
        acked_at: row.get(13)?,
        last_error: row.get(14)?,
        run_id: row.get(15)?,
        created_at: row.get(16)?,
        updated_at: row.get(17)?,
    })
}

fn parse_required<T>(
    index: usize,
    value: String,
    parse: impl FnOnce(&str) -> Option<T>,
) -> rusqlite::Result<T> {
    parse(&value).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(IoError::new(
                ErrorKind::InvalidData,
                format!("invalid enum value: {value}"),
            )),
        )
    })
}
