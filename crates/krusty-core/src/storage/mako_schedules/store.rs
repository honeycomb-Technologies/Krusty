use std::io::{Error as IoError, ErrorKind};

use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension, Row, Transaction, TransactionBehavior};

use crate::mako::{
    normalize_timestamp, parse_timezone, DstFoldPolicy, DstGapPolicy, DstPolicy, MisfireConfig,
    MisfirePolicy, RecurrenceV1, RetryJitter, RetryPolicy,
};
use crate::storage::Database;

use super::{
    MakoSchedule, MakoScheduleOccurrence, MakoScheduleOccurrenceStatus, MakoScheduleStatus,
    OverlapPolicy,
};

const SCHEDULE_COLUMNS: &str = "id, controller_id, title, summary, objective, recurrence_kind, recurrence_json, timezone, gap_policy, fold_policy, next_fire_at, last_scheduled_for, status, priority, project_dir, model, crew_slug, misfire_policy, misfire_grace_secs, catch_up_limit, overlap_policy, max_attempts, retry_base_secs, retry_max_secs, retry_jitter, revision, created_by, created_at, updated_at";
const OCCURRENCE_COLUMNS: &str = "id, schedule_id, scheduled_for, run_id, status, decision_reason, coalesced_count, created_at, updated_at";

pub struct MakoScheduleStore {
    db: Database,
}

impl MakoScheduleStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub fn insert_schedule(&self, schedule: &MakoSchedule) -> Result<()> {
        validate_schedule(schedule)?;
        let recurrence_json = serde_json::to_string(&schedule.recurrence)?;
        let next_fire_at = normalize_optional_timestamp(schedule.next_fire_at.as_deref())?;
        let last_scheduled_for =
            normalize_optional_timestamp(schedule.last_scheduled_for.as_deref())?;
        let created_at = normalize_timestamp(&schedule.created_at)?;
        let updated_at = normalize_timestamp(&schedule.updated_at)?;
        self.db.conn().execute(
            "INSERT INTO mako_schedules (
                id, controller_id, title, summary, objective, recurrence_kind,
                recurrence_json, timezone, gap_policy, fold_policy, next_fire_at,
                last_scheduled_for, status, priority, project_dir, model, crew_slug,
                misfire_policy, misfire_grace_secs, catch_up_limit, overlap_policy,
                max_attempts, retry_base_secs, retry_max_secs, retry_jitter,
                revision, created_by, created_at, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24,
                ?25, ?26, ?27, ?28, ?29
             )",
            params![
                schedule.id,
                schedule.controller_id,
                schedule.title,
                schedule.summary,
                schedule.objective,
                schedule.recurrence.kind_name(),
                recurrence_json,
                schedule.timezone,
                schedule.dst_policy.gap.as_str(),
                schedule.dst_policy.fold.as_str(),
                next_fire_at,
                last_scheduled_for,
                schedule.status.to_string(),
                schedule.priority,
                schedule.project_dir,
                schedule.model,
                schedule.crew_slug,
                schedule.misfire.policy.as_str(),
                schedule.misfire.grace_secs,
                schedule.misfire.catch_up_limit as u64,
                schedule.overlap_policy.as_str(),
                schedule.retry.max_attempts,
                schedule.retry.base_delay_secs,
                schedule.retry.max_delay_secs,
                schedule.retry.jitter.as_str(),
                schedule.revision,
                schedule.created_by,
                created_at,
                updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn get_schedule(&self, id: &str) -> Result<Option<MakoSchedule>> {
        let sql = format!("SELECT {SCHEDULE_COLUMNS} FROM mako_schedules WHERE id = ?1");
        self.db
            .conn()
            .query_row(&sql, [id], map_schedule)
            .optional()
            .context("reading Mako schedule")
    }

    pub fn list_due(&self, now: &str, limit: usize) -> Result<Vec<MakoSchedule>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let now = normalize_timestamp(now)?;
        let sql = format!(
            "SELECT {SCHEDULE_COLUMNS}
             FROM mako_schedules
             WHERE status = 'enabled'
               AND next_fire_at IS NOT NULL
               AND next_fire_at <= ?1
             ORDER BY next_fire_at ASC, created_at ASC
             LIMIT ?2"
        );
        let mut statement = self.db.conn().prepare(&sql)?;
        statement
            .query_map(params![now, limit as i64], map_schedule)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("listing due Mako schedules")
    }

    pub fn advance_schedule(
        &self,
        id: &str,
        expected_revision: u64,
        last_scheduled_for: &str,
        next_fire_at: Option<&str>,
        updated_at: &str,
    ) -> Result<bool> {
        let last_scheduled_for = normalize_timestamp(last_scheduled_for)?;
        let next_fire_at = normalize_optional_timestamp(next_fire_at)?;
        let updated_at = normalize_timestamp(updated_at)?;
        let changed = self.db.conn().execute(
            "UPDATE mako_schedules
             SET last_scheduled_for = ?3,
                 next_fire_at = ?4,
                 revision = revision + 1,
                 updated_at = ?5
             WHERE id = ?1 AND revision = ?2 AND status = 'enabled'",
            params![
                id,
                expected_revision,
                last_scheduled_for,
                next_fire_at,
                updated_at
            ],
        )?;
        Ok(changed == 1)
    }

    pub fn set_status(
        &self,
        id: &str,
        expected_revision: u64,
        status: MakoScheduleStatus,
        updated_at: &str,
    ) -> Result<bool> {
        let updated_at = normalize_timestamp(updated_at)?;
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let current = tx
            .query_row(
                "SELECT status, revision FROM mako_schedules WHERE id = ?1",
                [id],
                |row| Ok((row.get::<_, String>(0)?, nonnegative_i64(row, 1)? as u64)),
            )
            .optional()?;
        let Some((current, revision)) = current else {
            tx.commit()?;
            return Ok(false);
        };
        if revision != expected_revision {
            tx.commit()?;
            return Ok(false);
        }
        let current = MakoScheduleStatus::parse(&current)
            .ok_or_else(|| anyhow::anyhow!("invalid persisted Mako schedule status"))?;
        anyhow::ensure!(
            current.can_transition_to(status),
            "illegal Mako schedule transition from {current} to {status}"
        );
        let changed = tx.execute(
            "UPDATE mako_schedules
             SET status = ?3, revision = revision + 1, updated_at = ?4
             WHERE id = ?1 AND revision = ?2 AND status = ?5",
            params![
                id,
                expected_revision,
                status.to_string(),
                updated_at,
                current.to_string()
            ],
        )?;
        tx.commit()?;
        Ok(changed == 1)
    }

    /// Returns false when the logical `(schedule_id, scheduled_for)` already exists.
    pub fn insert_occurrence(&self, occurrence: &MakoScheduleOccurrence) -> Result<bool> {
        anyhow::ensure!(!occurrence.id.trim().is_empty(), "occurrence id is empty");
        anyhow::ensure!(
            !occurrence.schedule_id.trim().is_empty(),
            "occurrence schedule id is empty"
        );
        let scheduled_for = normalize_timestamp(&occurrence.scheduled_for)?;
        let created_at = normalize_timestamp(&occurrence.created_at)?;
        let updated_at = normalize_timestamp(&occurrence.updated_at)?;
        let changed = self.db.conn().execute(
            "INSERT INTO mako_schedule_occurrences (
                id, schedule_id, scheduled_for, run_id, status, decision_reason,
                coalesced_count, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(schedule_id, scheduled_for) DO NOTHING",
            params![
                occurrence.id,
                occurrence.schedule_id,
                scheduled_for,
                occurrence.run_id,
                occurrence.status.to_string(),
                occurrence.decision_reason,
                occurrence.coalesced_count,
                created_at,
                updated_at,
            ],
        )?;
        Ok(changed == 1)
    }

    pub fn get_occurrence(
        &self,
        schedule_id: &str,
        scheduled_for: &str,
    ) -> Result<Option<MakoScheduleOccurrence>> {
        let scheduled_for = normalize_timestamp(scheduled_for)?;
        let sql = format!(
            "SELECT {OCCURRENCE_COLUMNS}
             FROM mako_schedule_occurrences
             WHERE schedule_id = ?1 AND scheduled_for = ?2"
        );
        self.db
            .conn()
            .query_row(&sql, params![schedule_id, scheduled_for], map_occurrence)
            .optional()
            .context("reading Mako schedule occurrence")
    }
}

fn validate_schedule(schedule: &MakoSchedule) -> Result<()> {
    parse_timezone(&schedule.timezone)?;
    schedule.recurrence.validate()?;
    anyhow::ensure!(!schedule.id.trim().is_empty(), "schedule id is empty");
    anyhow::ensure!(
        !schedule.controller_id.trim().is_empty(),
        "schedule controller id is empty"
    );
    anyhow::ensure!(!schedule.title.trim().is_empty(), "schedule title is empty");
    anyhow::ensure!(
        !schedule.objective.trim().is_empty(),
        "schedule objective is empty"
    );
    anyhow::ensure!(schedule.retry.max_attempts > 0, "max_attempts is zero");
    anyhow::ensure!(
        schedule.retry.max_delay_secs >= schedule.retry.base_delay_secs,
        "retry maximum delay is less than base delay"
    );
    anyhow::ensure!(
        !schedule.created_by.trim().is_empty(),
        "schedule creator is empty"
    );
    Ok(())
}

fn normalize_optional_timestamp(value: Option<&str>) -> Result<Option<String>> {
    value
        .map(normalize_timestamp)
        .transpose()
        .map_err(Into::into)
}

fn map_schedule(row: &Row<'_>) -> rusqlite::Result<MakoSchedule> {
    let recurrence_kind: String = row.get(5)?;
    let recurrence_json: String = row.get(6)?;
    let recurrence = serde_json::from_str::<RecurrenceV1>(&recurrence_json)
        .map_err(|error| conversion_error(6, format!("invalid recurrence JSON: {error}")))?;
    if recurrence.kind_name() != recurrence_kind {
        return Err(conversion_error(
            5,
            format!(
                "recurrence kind {recurrence_kind} disagrees with {} payload",
                recurrence.kind_name()
            ),
        ));
    }

    let gap = parse_required(8, row.get::<_, String>(8)?, DstGapPolicy::parse)?;
    let fold = parse_required(9, row.get::<_, String>(9)?, DstFoldPolicy::parse)?;
    let status = parse_required(12, row.get::<_, String>(12)?, MakoScheduleStatus::parse)?;
    let misfire_policy = parse_required(17, row.get::<_, String>(17)?, MisfirePolicy::parse)?;
    let overlap_policy = parse_required(20, row.get::<_, String>(20)?, OverlapPolicy::parse)?;
    let retry_jitter = parse_required(24, row.get::<_, String>(24)?, RetryJitter::parse)?;

    Ok(MakoSchedule {
        id: row.get(0)?,
        controller_id: row.get(1)?,
        title: row.get(2)?,
        summary: row.get(3)?,
        objective: row.get(4)?,
        recurrence,
        timezone: row.get(7)?,
        dst_policy: DstPolicy { gap, fold },
        next_fire_at: row.get(10)?,
        last_scheduled_for: row.get(11)?,
        status,
        priority: row.get(13)?,
        project_dir: row.get(14)?,
        model: row.get(15)?,
        crew_slug: row.get(16)?,
        misfire: MisfireConfig {
            policy: misfire_policy,
            grace_secs: nonnegative_i64(row, 18)? as u64,
            catch_up_limit: nonnegative_i64(row, 19)? as usize,
        },
        overlap_policy,
        retry: RetryPolicy {
            max_attempts: nonnegative_i64(row, 21)? as u32,
            base_delay_secs: nonnegative_i64(row, 22)? as u64,
            max_delay_secs: nonnegative_i64(row, 23)? as u64,
            jitter: retry_jitter,
        },
        revision: nonnegative_i64(row, 25)? as u64,
        created_by: row.get(26)?,
        created_at: row.get(27)?,
        updated_at: row.get(28)?,
    })
}

fn map_occurrence(row: &Row<'_>) -> rusqlite::Result<MakoScheduleOccurrence> {
    let status = parse_required(
        4,
        row.get::<_, String>(4)?,
        MakoScheduleOccurrenceStatus::parse,
    )?;
    Ok(MakoScheduleOccurrence {
        id: row.get(0)?,
        schedule_id: row.get(1)?,
        scheduled_for: row.get(2)?,
        run_id: row.get(3)?,
        status,
        decision_reason: row.get(5)?,
        coalesced_count: nonnegative_i64(row, 6)? as u32,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

fn nonnegative_i64(row: &Row<'_>, index: usize) -> rusqlite::Result<i64> {
    let value = row.get::<_, i64>(index)?;
    if value < 0 {
        Err(conversion_error(index, "negative unsigned value"))
    } else {
        Ok(value)
    }
}

fn parse_required<T>(
    index: usize,
    value: String,
    parse: impl FnOnce(&str) -> Option<T>,
) -> rusqlite::Result<T> {
    parse(&value).ok_or_else(|| conversion_error(index, format!("invalid enum value: {value}")))
}

fn conversion_error(index: usize, message: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        index,
        rusqlite::types::Type::Text,
        Box::new(IoError::new(ErrorKind::InvalidData, message.into())),
    )
}
