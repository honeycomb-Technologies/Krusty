use std::io::{Error as IoError, ErrorKind};

use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension, Row, Transaction, TransactionBehavior};

use crate::ai::models::ModelKey;
use crate::hive::{
    normalize_timestamp, parse_timezone, DstFoldPolicy, DstGapPolicy, DstPolicy, MisfireConfig,
    MisfirePolicy, RecurrenceV1, RetryJitter, RetryPolicy,
};
use crate::storage::Database;

use super::{
    HiveSchedule, HiveScheduleOccurrence, HiveScheduleOccurrenceStatus, HiveScheduleStatus,
    OverlapPolicy, OwnedHiveSchedule,
};

const SCHEDULE_COLUMNS: &str = "id, controller_id, title, summary, objective, recurrence_kind, recurrence_json, timezone, gap_policy, fold_policy, next_fire_at, last_scheduled_for, status, priority, project_dir, model, model_key_json, model_catalog_revision, crew_slug, misfire_policy, misfire_grace_secs, catch_up_limit, overlap_policy, max_attempts, retry_base_secs, retry_max_secs, retry_jitter, revision, created_by, created_at, updated_at";
const OCCURRENCE_COLUMNS: &str = "id, schedule_id, scheduled_for, run_id, status, decision_reason, coalesced_count, created_at, updated_at";

pub struct HiveScheduleStore {
    db: Database,
}

impl HiveScheduleStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub fn insert_schedule(&self, schedule: &HiveSchedule) -> Result<()> {
        validate_schedule(schedule)?;
        let recurrence_json = serde_json::to_string(&schedule.recurrence)?;
        let next_fire_at = normalize_optional_timestamp(schedule.next_fire_at.as_deref())?;
        let last_scheduled_for =
            normalize_optional_timestamp(schedule.last_scheduled_for.as_deref())?;
        let created_at = normalize_timestamp(&schedule.created_at)?;
        let updated_at = normalize_timestamp(&schedule.updated_at)?;
        let model_key_json = schedule
            .model_key
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        self.db.conn().execute(
            "INSERT INTO hive_schedules (
                id, controller_id, title, summary, objective, recurrence_kind,
                recurrence_json, timezone, gap_policy, fold_policy, next_fire_at,
                last_scheduled_for, status, priority, project_dir, model,
                model_key_json, model_catalog_revision, crew_slug,
                misfire_policy, misfire_grace_secs, catch_up_limit, overlap_policy,
                max_attempts, retry_base_secs, retry_max_secs, retry_jitter,
                revision, created_by, created_at, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24,
                ?25, ?26, ?27, ?28, ?29, ?30, ?31
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
                model_key_json,
                schedule.model_catalog_revision,
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

    pub fn get_schedule(&self, id: &str) -> Result<Option<HiveSchedule>> {
        let sql = format!("SELECT {SCHEDULE_COLUMNS} FROM hive_schedules WHERE id = ?1");
        self.db
            .conn()
            .query_row(&sql, [id], map_schedule)
            .optional()
            .context("reading Hive schedule")
    }

    pub fn list_for_controller(
        &self,
        controller_id: &str,
        limit: usize,
    ) -> Result<Vec<HiveSchedule>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let sql = format!(
            "SELECT {SCHEDULE_COLUMNS}
             FROM hive_schedules
             WHERE controller_id = ?1
             ORDER BY created_at DESC, id ASC
             LIMIT ?2"
        );
        let mut statement = self.db.conn().prepare(&sql)?;
        let rows = statement
            .query_map(params![controller_id, limit as i64], map_schedule)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("listing Hive schedules for controller")?;
        Ok(rows)
    }

    /// List schedules owned by a user across all of their controllers.
    ///
    /// Ownership is resolved through `hive_controllers.user_id`. When
    /// `user_id` is `None`, only controllers with a NULL owner are included
    /// (local single-tenant mode).
    pub fn list_for_user(
        &self,
        user_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<OwnedHiveSchedule>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let sql =
            "SELECT s.id, s.controller_id, s.title, s.summary, s.objective, s.recurrence_kind, s.recurrence_json, s.timezone, s.gap_policy, s.fold_policy, s.next_fire_at, s.last_scheduled_for, s.status, s.priority, s.project_dir, s.model, s.model_key_json, s.model_catalog_revision, s.crew_slug, s.misfire_policy, s.misfire_grace_secs, s.catch_up_limit, s.overlap_policy, s.max_attempts, s.retry_base_secs, s.retry_max_secs, s.retry_jitter, s.revision, s.created_by, s.created_at, s.updated_at, c.session_id
             FROM hive_schedules s
             INNER JOIN hive_controllers c ON c.id = s.controller_id
             WHERE ((?1 IS NULL AND c.user_id IS NULL) OR c.user_id = ?1)
             ORDER BY
               CASE WHEN s.next_fire_at IS NULL THEN 1 ELSE 0 END,
               s.next_fire_at ASC,
               s.created_at DESC,
               s.id ASC
             LIMIT ?2";
        let mut statement = self.db.conn().prepare(sql)?;
        let rows = statement
            .query_map(params![user_id, limit as i64], |row| {
                Ok(OwnedHiveSchedule {
                    schedule: map_schedule(row)?,
                    controller_session_id: row.get(31)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("listing Hive schedules for user")?;
        Ok(rows)
    }

    pub fn list_due(&self, now: &str, limit: usize) -> Result<Vec<HiveSchedule>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let now = normalize_timestamp(now)?;
        let sql = format!(
            "SELECT {SCHEDULE_COLUMNS}
             FROM hive_schedules
             WHERE status = 'enabled'
               AND next_fire_at IS NOT NULL
               AND next_fire_at <= ?1
             ORDER BY next_fire_at ASC, created_at ASC
             LIMIT ?2"
        );
        let mut statement = self.db.conn().prepare(&sql)?;
        let schedules = statement
            .query_map(params![now, limit as i64], map_schedule)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("listing due Hive schedules")?;
        Ok(schedules)
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
            "UPDATE hive_schedules
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
        status: HiveScheduleStatus,
        updated_at: &str,
    ) -> Result<bool> {
        let updated_at = normalize_timestamp(updated_at)?;
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let current = tx
            .query_row(
                "SELECT status, revision FROM hive_schedules WHERE id = ?1",
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
        let current = HiveScheduleStatus::parse(&current)
            .ok_or_else(|| anyhow::anyhow!("invalid persisted Hive schedule status"))?;
        anyhow::ensure!(
            current.can_transition_to(status),
            "illegal Hive schedule transition from {current} to {status}"
        );
        let changed = tx.execute(
            "UPDATE hive_schedules
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
    pub fn insert_occurrence(&self, occurrence: &HiveScheduleOccurrence) -> Result<bool> {
        anyhow::ensure!(!occurrence.id.trim().is_empty(), "occurrence id is empty");
        anyhow::ensure!(
            !occurrence.schedule_id.trim().is_empty(),
            "occurrence schedule id is empty"
        );
        let scheduled_for = normalize_timestamp(&occurrence.scheduled_for)?;
        let created_at = normalize_timestamp(&occurrence.created_at)?;
        let updated_at = normalize_timestamp(&occurrence.updated_at)?;
        let changed = self.db.conn().execute(
            "INSERT INTO hive_schedule_occurrences (
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
    ) -> Result<Option<HiveScheduleOccurrence>> {
        let scheduled_for = normalize_timestamp(scheduled_for)?;
        let sql = format!(
            "SELECT {OCCURRENCE_COLUMNS}
             FROM hive_schedule_occurrences
             WHERE schedule_id = ?1 AND scheduled_for = ?2"
        );
        self.db
            .conn()
            .query_row(&sql, params![schedule_id, scheduled_for], map_occurrence)
            .optional()
            .context("reading Hive schedule occurrence")
    }

    pub fn list_occurrences(
        &self,
        schedule_id: &str,
        limit: usize,
    ) -> Result<Vec<HiveScheduleOccurrence>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let sql = format!(
            "SELECT {OCCURRENCE_COLUMNS}
             FROM hive_schedule_occurrences
             WHERE schedule_id = ?1
             ORDER BY scheduled_for DESC, id ASC
             LIMIT ?2"
        );
        let mut statement = self.db.conn().prepare(&sql)?;
        let rows = statement
            .query_map(params![schedule_id, limit as i64], map_occurrence)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("listing Hive schedule occurrences")?;
        Ok(rows)
    }
}

fn validate_schedule(schedule: &HiveSchedule) -> Result<()> {
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
    if let Some(key) = schedule.model_key.as_ref() {
        anyhow::ensure!(
            schedule.model.as_deref() == Some(key.model_id.as_str()),
            "schedule model does not match model key"
        );
    } else {
        anyhow::ensure!(
            schedule.model_catalog_revision.is_none(),
            "schedule catalog revision requires a model key"
        );
    }
    Ok(())
}

fn normalize_optional_timestamp(value: Option<&str>) -> Result<Option<String>> {
    value
        .map(normalize_timestamp)
        .transpose()
        .map_err(Into::into)
}

fn map_schedule(row: &Row<'_>) -> rusqlite::Result<HiveSchedule> {
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
    let status = parse_required(12, row.get::<_, String>(12)?, HiveScheduleStatus::parse)?;
    let model_key = row
        .get::<_, Option<String>>(16)?
        .map(|value| {
            serde_json::from_str::<ModelKey>(&value)
                .map_err(|error| conversion_error(16, format!("invalid model key JSON: {error}")))
        })
        .transpose()?;
    let misfire_policy = parse_required(19, row.get::<_, String>(19)?, MisfirePolicy::parse)?;
    let overlap_policy = parse_required(22, row.get::<_, String>(22)?, OverlapPolicy::parse)?;
    let retry_jitter = parse_required(26, row.get::<_, String>(26)?, RetryJitter::parse)?;

    Ok(HiveSchedule {
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
        model_key,
        model_catalog_revision: row.get(17)?,
        crew_slug: row.get(18)?,
        misfire: MisfireConfig {
            policy: misfire_policy,
            grace_secs: nonnegative_i64(row, 20)? as u64,
            catch_up_limit: nonnegative_i64(row, 21)? as usize,
        },
        overlap_policy,
        retry: RetryPolicy {
            max_attempts: nonnegative_i64(row, 23)? as u32,
            base_delay_secs: nonnegative_i64(row, 24)? as u64,
            max_delay_secs: nonnegative_i64(row, 25)? as u64,
            jitter: retry_jitter,
        },
        revision: nonnegative_i64(row, 27)? as u64,
        created_by: row.get(28)?,
        created_at: row.get(29)?,
        updated_at: row.get(30)?,
    })
}

fn map_occurrence(row: &Row<'_>) -> rusqlite::Result<HiveScheduleOccurrence> {
    let status = parse_required(
        4,
        row.get::<_, String>(4)?,
        HiveScheduleOccurrenceStatus::parse,
    )?;
    Ok(HiveScheduleOccurrence {
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
