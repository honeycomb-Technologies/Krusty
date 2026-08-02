use anyhow::{ensure, Result};
use chrono::{Duration, Utc};
use rusqlite::{params, OptionalExtension, TransactionBehavior};

use crate::storage::Database;

use super::{
    MobileDiagnosticCategoryCount, MobileDiagnosticEvent, MobileDiagnosticEventInput,
    MobileDiagnosticNativePayload, MobileDiagnosticNativePayloadInput, MobileDiagnosticReport,
    MobileDiagnosticRun, MobileDiagnosticRunInput,
};

const RUN_COLUMNS: &str = "id, user_id, installation_id, app_version, build_number, platform, \
    os_version, device_class, capture_level, started_at_ms, ended_at_ms, status, event_count, \
    dropped_event_count, byte_count, created_at, updated_at";
const MAX_EVENTS_PER_RUN: i64 = 10_000;
const MAX_NATIVE_PAYLOADS_PER_RUN: i64 = 32;

pub struct MobileDiagnosticStore {
    db: Database,
}

impl MobileDiagnosticStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub fn ingest_batch(
        &mut self,
        run: MobileDiagnosticRunInput<'_>,
        events: &[MobileDiagnosticEventInput<'_>],
        native_payloads: &[MobileDiagnosticNativePayloadInput<'_>],
    ) -> Result<(usize, usize)> {
        let now = Utc::now().to_rfc3339();
        // Acquire the write reservation before checking ownership. Keeping the
        // check and upsert under one IMMEDIATE transaction prevents two first
        // uploads with the same run id from both observing an absent row.
        let tx = self
            .db
            .conn_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing_owner = tx
            .query_row(
                "SELECT user_id, installation_id FROM mobile_diagnostic_runs WHERE id = ?1",
                [run.id],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        if let Some((owner, installation_id)) = existing_owner {
            ensure!(
                owner.as_deref() == run.user_id && installation_id == run.installation_id,
                "mobile diagnostic run ownership mismatch"
            );
        }
        tx.execute(
            "INSERT INTO mobile_diagnostic_runs (
                id, user_id, installation_id, app_version, build_number, platform,
                os_version, device_class, capture_level, started_at_ms, ended_at_ms,
                status, dropped_event_count, byte_count, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)
             ON CONFLICT(id) DO UPDATE SET
                capture_level = CASE
                    WHEN excluded.capture_level = 'stress' THEN 'stress'
                    ELSE mobile_diagnostic_runs.capture_level
                END,
                ended_at_ms = COALESCE(excluded.ended_at_ms, mobile_diagnostic_runs.ended_at_ms),
                status = CASE WHEN excluded.status = 'completed' THEN 'completed' ELSE mobile_diagnostic_runs.status END,
                dropped_event_count = MAX(mobile_diagnostic_runs.dropped_event_count, excluded.dropped_event_count),
                updated_at = excluded.updated_at",
            params![
                run.id,
                run.user_id,
                run.installation_id,
                run.app_version,
                run.build_number,
                run.platform,
                run.os_version,
                run.device_class,
                run.capture_level,
                run.started_at_ms,
                run.ended_at_ms,
                if run.completed { "completed" } else { "active" },
                run.dropped_event_count as i64,
                0_i64,
                now,
            ],
        )?;

        let mut inserted = 0usize;
        let mut inserted_bytes = 0usize;
        let mut inserted_native_payloads = 0usize;
        {
            let mut statement = tx.prepare(
                "INSERT OR IGNORE INTO mobile_diagnostic_events (
                    run_id, sequence, occurred_at_ms, monotonic_ms, category, name,
                    duration_ms, severity, attributes_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for event in events {
                let accepted = statement.execute(params![
                    run.id,
                    event.sequence as i64,
                    event.occurred_at_ms,
                    event.monotonic_ms,
                    event.category,
                    event.name,
                    event.duration_ms,
                    event.severity,
                    event.attributes_json,
                ])?;
                inserted += accepted;
                if accepted > 0 {
                    inserted_bytes += event.category.len()
                        + event.name.len()
                        + event.severity.len()
                        + event.attributes_json.len()
                        + 64;
                }
            }
        }
        {
            let mut statement = tx.prepare(
                "INSERT OR IGNORE INTO mobile_diagnostic_native_payloads (
                    run_id, payload_id, kind, received_at_ms, payload_json, byte_count
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for payload in native_payloads {
                let accepted = statement.execute(params![
                    run.id,
                    payload.payload_id,
                    payload.kind,
                    payload.received_at_ms,
                    payload.payload_json,
                    payload.payload_json.len() as i64,
                ])?;
                if accepted > 0 {
                    inserted_native_payloads += 1;
                    inserted_bytes += payload.payload_json.len() + payload.kind.len() + 64;
                }
            }
        }
        tx.execute(
            "DELETE FROM mobile_diagnostic_events
             WHERE run_id = ?1 AND sequence NOT IN (
                 SELECT sequence FROM mobile_diagnostic_events
                 WHERE run_id = ?1 ORDER BY sequence DESC LIMIT ?2
             )",
            params![run.id, MAX_EVENTS_PER_RUN],
        )?;
        tx.execute(
            "DELETE FROM mobile_diagnostic_native_payloads
             WHERE run_id = ?1 AND payload_id NOT IN (
                 SELECT payload_id FROM mobile_diagnostic_native_payloads
                 WHERE run_id = ?1 ORDER BY received_at_ms DESC LIMIT ?2
             )",
            params![run.id, MAX_NATIVE_PAYLOADS_PER_RUN],
        )?;
        tx.execute(
            "UPDATE mobile_diagnostic_runs
             SET event_count = (
                 SELECT COUNT(*) FROM mobile_diagnostic_events WHERE run_id = ?1
             ), byte_count = byte_count + ?2
             WHERE id = ?1",
            params![run.id, inserted_bytes as i64],
        )?;
        tx.commit()?;
        Ok((inserted, inserted_native_payloads))
    }

    pub fn prune_older_than_days(&self, days: i64) -> Result<usize> {
        let threshold = (Utc::now() - Duration::days(days)).to_rfc3339();
        Ok(self.db.conn().execute(
            "DELETE FROM mobile_diagnostic_runs WHERE updated_at < ?1",
            [threshold],
        )?)
    }

    pub fn list_runs_for_user(
        &self,
        user_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MobileDiagnosticRun>> {
        let limit = limit.clamp(1, 100) as i64;
        let sql = match user_id {
            Some(_) => format!(
                "SELECT {RUN_COLUMNS} FROM mobile_diagnostic_runs
                 WHERE user_id = ?1 ORDER BY updated_at DESC LIMIT ?2"
            ),
            None => format!(
                "SELECT {RUN_COLUMNS} FROM mobile_diagnostic_runs
                 WHERE user_id IS NULL ORDER BY updated_at DESC LIMIT ?1"
            ),
        };
        let mut statement = self.db.conn().prepare(&sql)?;
        let rows = match user_id {
            Some(value) => statement.query_map(params![value, limit], map_run)?,
            None => statement.query_map([limit], map_run)?,
        };
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn report_for_user(
        &self,
        run_id: &str,
        user_id: Option<&str>,
    ) -> Result<Option<MobileDiagnosticReport>> {
        let sql = match user_id {
            Some(_) => format!(
                "SELECT {RUN_COLUMNS} FROM mobile_diagnostic_runs WHERE id = ?1 AND user_id = ?2"
            ),
            None => format!(
                "SELECT {RUN_COLUMNS} FROM mobile_diagnostic_runs WHERE id = ?1 AND user_id IS NULL"
            ),
        };
        let run = match user_id {
            Some(value) => self
                .db
                .conn()
                .query_row(&sql, params![run_id, value], map_run)
                .optional()?,
            None => self
                .db
                .conn()
                .query_row(&sql, [run_id], map_run)
                .optional()?,
        };
        let Some(run) = run else {
            return Ok(None);
        };

        let categories = self.category_counts(run_id)?;
        let recent_events = self.recent_events(run_id, 200)?;
        let (long_task_count, max_long_task_ms) =
            self.metric_summary(run_id, "runtime", "long_task")?;
        let (heartbeat_stall_count, max_heartbeat_drift_ms) =
            self.metric_summary(run_id, "runtime", "heartbeat_stall")?;
        let (webview_termination_count, _) =
            self.metric_summary(run_id, "webview", "terminated")?;
        let error_count: i64 = self.db.conn().query_row(
            "SELECT COUNT(*) FROM mobile_diagnostic_events
             WHERE run_id = ?1 AND severity IN ('error', 'fatal')",
            [run_id],
            |row| row.get(0),
        )?;
        let native_payload_count: i64 = self.db.conn().query_row(
            "SELECT COUNT(*) FROM mobile_diagnostic_native_payloads WHERE run_id = ?1",
            [run_id],
            |row| row.get(0),
        )?;

        Ok(Some(MobileDiagnosticReport {
            run,
            categories,
            long_task_count,
            max_long_task_ms,
            heartbeat_stall_count,
            max_heartbeat_drift_ms,
            webview_termination_count,
            error_count: error_count as usize,
            native_payload_count: native_payload_count as usize,
            recent_events,
        }))
    }

    pub fn native_payloads_for_user(
        &self,
        run_id: &str,
        user_id: Option<&str>,
    ) -> Result<Option<Vec<MobileDiagnosticNativePayload>>> {
        let owned: bool = match user_id {
            Some(value) => self.db.conn().query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM mobile_diagnostic_runs WHERE id = ?1 AND user_id = ?2
                 )",
                params![run_id, value],
                |row| row.get(0),
            )?,
            None => self.db.conn().query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM mobile_diagnostic_runs WHERE id = ?1 AND user_id IS NULL
                 )",
                [run_id],
                |row| row.get(0),
            )?,
        };
        if !owned {
            return Ok(None);
        }

        let mut statement = self.db.conn().prepare(
            "SELECT run_id, payload_id, kind, received_at_ms, payload_json, byte_count
             FROM mobile_diagnostic_native_payloads WHERE run_id = ?1
             ORDER BY received_at_ms ASC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![run_id, MAX_NATIVE_PAYLOADS_PER_RUN], |row| {
            let payload_json: String = row.get(4)?;
            Ok(MobileDiagnosticNativePayload {
                run_id: row.get(0)?,
                payload_id: row.get(1)?,
                kind: row.get(2)?,
                received_at_ms: row.get(3)?,
                payload: serde_json::from_str(&payload_json).unwrap_or_default(),
                byte_count: row.get::<_, i64>(5)? as usize,
            })
        })?;
        Ok(Some(rows.collect::<rusqlite::Result<Vec<_>>>()?))
    }

    pub fn events_for_user(
        &self,
        run_id: &str,
        user_id: Option<&str>,
        after_sequence: u64,
        limit: usize,
    ) -> Result<Option<Vec<MobileDiagnosticEvent>>> {
        let owned: bool = match user_id {
            Some(value) => self.db.conn().query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM mobile_diagnostic_runs WHERE id = ?1 AND user_id = ?2
                 )",
                params![run_id, value],
                |row| row.get(0),
            )?,
            None => self.db.conn().query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM mobile_diagnostic_runs WHERE id = ?1 AND user_id IS NULL
                 )",
                [run_id],
                |row| row.get(0),
            )?,
        };
        if !owned {
            return Ok(None);
        }

        let mut statement = self.db.conn().prepare(
            "SELECT run_id, sequence, occurred_at_ms, monotonic_ms, category, name,
                    duration_ms, severity, attributes_json
             FROM mobile_diagnostic_events
             WHERE run_id = ?1 AND sequence > ?2
             ORDER BY sequence ASC LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![run_id, after_sequence as i64, limit.clamp(1, 501) as i64],
            map_event,
        )?;
        Ok(Some(rows.collect::<rusqlite::Result<Vec<_>>>()?))
    }

    fn category_counts(&self, run_id: &str) -> Result<Vec<MobileDiagnosticCategoryCount>> {
        let mut statement = self.db.conn().prepare(
            "SELECT category, COUNT(*) FROM mobile_diagnostic_events
             WHERE run_id = ?1 GROUP BY category ORDER BY COUNT(*) DESC, category ASC",
        )?;
        let rows = statement.query_map([run_id], |row| {
            Ok(MobileDiagnosticCategoryCount {
                category: row.get(0)?,
                count: row.get::<_, i64>(1)? as usize,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    fn metric_summary(
        &self,
        run_id: &str,
        category: &str,
        name: &str,
    ) -> Result<(usize, Option<f64>)> {
        let (count, maximum): (i64, Option<f64>) = self.db.conn().query_row(
            "SELECT COUNT(*), MAX(duration_ms) FROM mobile_diagnostic_events
             WHERE run_id = ?1 AND category = ?2 AND name = ?3",
            params![run_id, category, name],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok((count as usize, maximum))
    }

    fn recent_events(&self, run_id: &str, limit: usize) -> Result<Vec<MobileDiagnosticEvent>> {
        let mut statement = self.db.conn().prepare(
            "SELECT run_id, sequence, occurred_at_ms, monotonic_ms, category, name,
                    duration_ms, severity, attributes_json
             FROM mobile_diagnostic_events WHERE run_id = ?1
             ORDER BY sequence DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![run_id, limit as i64], map_event)?;
        let mut events = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        events.reverse();
        Ok(events)
    }
}

fn map_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<MobileDiagnosticEvent> {
    let attributes_json: String = row.get(8)?;
    let attributes = serde_json::from_str(&attributes_json).unwrap_or_default();
    Ok(MobileDiagnosticEvent {
        run_id: row.get(0)?,
        sequence: row.get::<_, i64>(1)? as u64,
        occurred_at_ms: row.get(2)?,
        monotonic_ms: row.get(3)?,
        category: row.get(4)?,
        name: row.get(5)?,
        duration_ms: row.get(6)?,
        severity: row.get(7)?,
        attributes,
    })
}

fn map_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<MobileDiagnosticRun> {
    Ok(MobileDiagnosticRun {
        id: row.get(0)?,
        user_id: row.get(1)?,
        installation_id: row.get(2)?,
        app_version: row.get(3)?,
        build_number: row.get(4)?,
        platform: row.get(5)?,
        os_version: row.get(6)?,
        device_class: row.get(7)?,
        capture_level: row.get(8)?,
        started_at_ms: row.get(9)?,
        ended_at_ms: row.get(10)?,
        status: row.get(11)?,
        event_count: row.get::<_, i64>(12)? as usize,
        dropped_event_count: row.get::<_, i64>(13)? as usize,
        byte_count: row.get::<_, i64>(14)? as usize,
        created_at: row.get(15)?,
        updated_at: row.get(16)?,
    })
}

#[cfg(test)]
mod ownership_tests {
    use std::sync::{Arc, Barrier};
    use std::thread;

    use tempfile::TempDir;

    use super::*;

    fn run_for(owner: &'static str) -> MobileDiagnosticRunInput<'static> {
        MobileDiagnosticRunInput {
            id: "run-shared",
            user_id: Some(owner),
            installation_id: if owner == "alice" {
                "install-alice"
            } else {
                "install-bob"
            },
            app_version: "0.9.20",
            build_number: "261",
            platform: "ios",
            os_version: "26.6",
            device_class: "iPhone",
            capture_level: "stress",
            started_at_ms: 1,
            ended_at_ms: None,
            completed: false,
            dropped_event_count: 0,
        }
    }

    fn event_for(owner: &'static str) -> MobileDiagnosticEventInput<'static> {
        MobileDiagnosticEventInput {
            sequence: if owner == "alice" { 1 } else { 2 },
            occurred_at_ms: 2,
            monotonic_ms: 1.0,
            category: "runtime",
            name: if owner == "alice" {
                "alice_event"
            } else {
                "bob_event"
            },
            duration_ms: None,
            severity: "info",
            attributes_json: "{}",
        }
    }

    #[test]
    fn concurrent_first_uploads_cannot_cross_run_ownership() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("diagnostics-race.db");
        // Complete migrations before the competing connections are opened.
        drop(Database::new(&path).expect("seed database"));

        let barrier = Arc::new(Barrier::new(2));
        let handles = ["alice", "bob"].map(|owner| {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let mut store = MobileDiagnosticStore::new(
                    Database::new(&path).expect("open competing database"),
                );
                barrier.wait();
                let event = event_for(owner);
                let result = store.ingest_batch(run_for(owner), &[event], &[]);
                (owner, result)
            })
        });

        let outcomes = handles.map(|handle| handle.join().expect("ingest thread"));
        let winners = outcomes
            .iter()
            .filter(|(_, result)| result.is_ok())
            .collect::<Vec<_>>();
        let losers = outcomes
            .iter()
            .filter(|(_, result)| result.is_err())
            .collect::<Vec<_>>();
        assert_eq!(winners.len(), 1, "exactly one owner must create the run");
        assert_eq!(losers.len(), 1, "the competing owner must be rejected");
        assert!(
            losers[0]
                .1
                .as_ref()
                .expect_err("loser error")
                .to_string()
                .contains("ownership mismatch"),
            "loser must fail the ownership check after acquiring the write lock"
        );

        let winner = winners[0].0;
        let loser = losers[0].0;
        let store = MobileDiagnosticStore::new(Database::new(&path).expect("reopen database"));
        let report = store
            .report_for_user("run-shared", Some(winner))
            .expect("winner report query")
            .expect("winner owns report");
        assert_eq!(report.run.user_id.as_deref(), Some(winner));
        assert_eq!(report.run.event_count, 1);
        assert_eq!(report.recent_events[0].name, event_for(winner).name);
        assert!(
            store
                .report_for_user("run-shared", Some(loser))
                .expect("loser report query")
                .is_none(),
            "losing owner must neither own nor inject into the run"
        );
    }
}
