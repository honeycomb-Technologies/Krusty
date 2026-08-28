use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction, TransactionBehavior};

use crate::storage::Database;

use super::{HiveGroupWorkerLane, NewHiveGroupWorkerLane};

const LANE_COLUMNS: &str = "group_id, worker_id, session_id, created_at, updated_at";

/// Storage for isolated `(group, Worker)` conversation sessions.
pub struct HiveGroupWorkerLaneStore {
    db: Database,
}

impl HiveGroupWorkerLaneStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub fn load(&self, group_id: &str, worker_id: &str) -> Result<Option<HiveGroupWorkerLane>> {
        load_group_worker_lane_with_conn(self.db.conn(), group_id, worker_id)
    }

    /// Insert a lane candidate or adopt the already persisted canonical lane.
    ///
    /// An immediate transaction serializes membership validation with the
    /// binding. The SQL conflict handler intentionally leaves a different
    /// existing `session_id` unchanged, so simultaneous creators converge on
    /// the first committed lane.
    pub fn upsert(&self, input: &NewHiveGroupWorkerLane) -> Result<HiveGroupWorkerLane> {
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)
            .context("acquiring Hive group Worker lane lock")?;
        let now = chrono::Utc::now().to_rfc3339();
        let lane = upsert_group_worker_lane_with_conn(&tx, input, &now)?;
        tx.commit()?;
        Ok(lane)
    }
}

pub fn load_group_worker_lane_with_conn(
    conn: &Connection,
    group_id: &str,
    worker_id: &str,
) -> Result<Option<HiveGroupWorkerLane>> {
    let sql = format!(
        "SELECT {LANE_COLUMNS} FROM hive_group_worker_lanes
         WHERE group_id = ?1 AND worker_id = ?2"
    );
    let lane = conn
        .query_row(&sql, params![group_id, worker_id], map_lane)
        .optional()
        .context("loading Hive group Worker lane")?;
    if let Some(lane) = &lane {
        validate_lane_binding(conn, &lane.group_id, &lane.worker_id, &lane.session_id)?;
    }
    Ok(lane)
}

/// Upsert a candidate using an existing connection or caller-owned
/// transaction and return the canonical row.
///
/// The helper validates that the Worker is currently a member, that the lane
/// session is an otherwise-unbound Hive session, and that group, Worker, and
/// session have the exact same owner. In particular a Worker's direct DM can
/// never become a group lane. Its conflict branch is a no-op for a different
/// candidate session, making the result deterministic under concurrent
/// creation.
pub fn upsert_group_worker_lane_with_conn(
    conn: &Connection,
    input: &NewHiveGroupWorkerLane,
    now: &str,
) -> Result<HiveGroupWorkerLane> {
    ensure!(!input.group_id.trim().is_empty(), "Hive group id is empty");
    ensure!(
        !input.worker_id.trim().is_empty(),
        "Hive Worker id is empty"
    );
    ensure!(
        !input.session_id.trim().is_empty(),
        "Hive lane session id is empty"
    );
    ensure!(!now.trim().is_empty(), "Hive lane timestamp is empty");

    validate_lane_binding(conn, &input.group_id, &input.worker_id, &input.session_id)?;

    let sql = format!(
        "INSERT INTO hive_group_worker_lanes (
             group_id, worker_id, session_id, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT(group_id, worker_id) DO UPDATE SET
             updated_at = CASE
                 WHEN hive_group_worker_lanes.session_id = excluded.session_id
                 THEN excluded.updated_at
                 ELSE hive_group_worker_lanes.updated_at
             END
         RETURNING {LANE_COLUMNS}"
    );
    let lane = conn
        .query_row(
            &sql,
            params![input.group_id, input.worker_id, input.session_id, now],
            map_lane,
        )
        .context("upserting Hive group Worker lane")?;
    // The conflict branch adopts the existing row. Revalidate that canonical
    // binding as well so a malformed legacy row cannot bypass the candidate
    // checks and silently route a group run through a direct DM.
    validate_lane_binding(conn, &lane.group_id, &lane.worker_id, &lane.session_id)?;
    Ok(lane)
}

fn validate_lane_binding(
    conn: &Connection,
    group_id: &str,
    worker_id: &str,
    session_id: &str,
) -> Result<()> {
    let valid: bool = conn.query_row(
        "SELECT EXISTS(
             SELECT 1
             FROM hive_group_members member
             JOIN hive_groups group_row ON group_row.id = member.group_id
             JOIN hive_workers worker ON worker.id = member.worker_id
             JOIN sessions session ON session.id = ?3
             WHERE member.group_id = ?1
               AND member.worker_id = ?2
               AND session.session_type = 'hive'
               AND NOT EXISTS (
                   SELECT 1 FROM hive_workers dm_owner
                   WHERE dm_owner.dm_session_id = session.id
               )
               AND (
                   (group_row.user_id IS NULL AND worker.user_id IS NULL)
                   OR group_row.user_id = worker.user_id
               )
               AND (
                   (group_row.user_id IS NULL AND session.user_id IS NULL)
                   OR group_row.user_id = session.user_id
               )
         )",
        params![group_id, worker_id, session_id],
        |row| row.get(0),
    )?;
    ensure!(
        valid,
        "Hive group Worker lane requires a same-owner member and a non-DM Hive session"
    );
    Ok(())
}

fn map_lane(row: &Row<'_>) -> rusqlite::Result<HiveGroupWorkerLane> {
    Ok(HiveGroupWorkerLane {
        group_id: row.get(0)?,
        worker_id: row.get(1)?,
        session_id: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}
