use anyhow::Result;
use chrono::{Duration, Utc};
use rusqlite::params;
use sha2::{Digest, Sha256};
use url::Url;

use crate::storage::database::Database;

use super::model::{
    AttemptOutcomeFilter, PushDeliveryAttempt, PushDeliveryAttemptInput, PushDeliverySummary,
};

const SELECT_ATTEMPT_COLUMNS: &str = r#"
    SELECT id, user_id, session_id, endpoint_hash, provider_host, event_type,
           outcome, http_status, error_message, latency_ms, created_at
    FROM push_delivery_attempts
"#;

pub struct PushDeliveryAttemptStore<'a> {
    db: &'a Database,
}

impl<'a> PushDeliveryAttemptStore<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn record_attempt(&self, input: PushDeliveryAttemptInput<'_>) -> Result<()> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let endpoint_hash = endpoint_hash(input.endpoint);
        let provider_host = provider_host(input.endpoint);
        let http_status = input.http_status.map(i64::from);
        let latency_ms = input.latency_ms.map(|v| v as i64);

        self.db.conn().execute(
            "INSERT INTO push_delivery_attempts (
                id, user_id, session_id, endpoint_hash, provider_host, event_type,
                outcome, http_status, error_message, latency_ms, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                id,
                input.user_id,
                input.session_id,
                endpoint_hash,
                provider_host,
                input.event_type,
                input.outcome,
                http_status,
                input.error_message,
                latency_ms,
                now
            ],
        )?;

        Ok(())
    }

    pub fn latest_for_user(&self, user_id: Option<&str>) -> Result<Option<PushDeliveryAttempt>> {
        let sql = match user_id {
            Some(_) => {
                format!(
                    "{SELECT_ATTEMPT_COLUMNS}
                     WHERE user_id = ?1
                     ORDER BY created_at DESC
                     LIMIT 1"
                )
            }
            None => {
                format!(
                    "{SELECT_ATTEMPT_COLUMNS}
                     ORDER BY created_at DESC
                     LIMIT 1"
                )
            }
        };

        let mut stmt = self.db.conn().prepare(&sql)?;
        let mut rows = match user_id {
            Some(uid) => stmt.query([uid])?,
            None => stmt.query([])?,
        };

        if let Some(row) = rows.next()? {
            Ok(Some(map_attempt_row(row)?))
        } else {
            Ok(None)
        }
    }

    pub fn summary_for_user(&self, user_id: Option<&str>) -> Result<PushDeliverySummary> {
        let last_attempt_at = self.latest_timestamp(user_id, AttemptOutcomeFilter::Any);
        let last_success_at = self.latest_timestamp(user_id, AttemptOutcomeFilter::Success);
        let last_failure_at = self.latest_timestamp(user_id, AttemptOutcomeFilter::Failure);
        let last_failure_reason = self.latest_failure_reason(user_id);

        let threshold = (Utc::now() - Duration::hours(24)).to_rfc3339();
        let recent_failures_24h: i64 = match user_id {
            Some(uid) => self.db.conn().query_row(
                "SELECT COUNT(*)
                 FROM push_delivery_attempts
                 WHERE user_id = ?1 AND outcome = 'failure' AND created_at >= ?2",
                params![uid, threshold],
                |row| row.get(0),
            )?,
            None => self.db.conn().query_row(
                "SELECT COUNT(*)
                 FROM push_delivery_attempts
                 WHERE outcome = 'failure' AND created_at >= ?1",
                params![threshold],
                |row| row.get(0),
            )?,
        };

        Ok(PushDeliverySummary {
            last_attempt_at,
            last_success_at,
            last_failure_at,
            last_failure_reason,
            recent_failures_24h: recent_failures_24h as usize,
        })
    }

    fn latest_timestamp(
        &self,
        user_id: Option<&str>,
        outcome_filter: AttemptOutcomeFilter,
    ) -> Option<String> {
        let result = match (user_id, outcome_filter) {
            (Some(uid), AttemptOutcomeFilter::Any) => self.db.conn().query_row(
                "SELECT created_at
                 FROM push_delivery_attempts
                 WHERE user_id = ?1
                 ORDER BY created_at DESC
                 LIMIT 1",
                [uid],
                |row| row.get(0),
            ),
            (Some(uid), AttemptOutcomeFilter::Success) => self.db.conn().query_row(
                "SELECT created_at
                 FROM push_delivery_attempts
                 WHERE user_id = ?1 AND outcome = 'success'
                 ORDER BY created_at DESC
                 LIMIT 1",
                [uid],
                |row| row.get(0),
            ),
            (Some(uid), AttemptOutcomeFilter::Failure) => self.db.conn().query_row(
                "SELECT created_at
                 FROM push_delivery_attempts
                 WHERE user_id = ?1 AND outcome = 'failure'
                 ORDER BY created_at DESC
                 LIMIT 1",
                [uid],
                |row| row.get(0),
            ),
            (None, AttemptOutcomeFilter::Any) => self.db.conn().query_row(
                "SELECT created_at
                 FROM push_delivery_attempts
                 ORDER BY created_at DESC
                 LIMIT 1",
                [],
                |row| row.get(0),
            ),
            (None, AttemptOutcomeFilter::Success) => self.db.conn().query_row(
                "SELECT created_at
                 FROM push_delivery_attempts
                 WHERE outcome = 'success'
                 ORDER BY created_at DESC
                 LIMIT 1",
                [],
                |row| row.get(0),
            ),
            (None, AttemptOutcomeFilter::Failure) => self.db.conn().query_row(
                "SELECT created_at
                 FROM push_delivery_attempts
                 WHERE outcome = 'failure'
                 ORDER BY created_at DESC
                 LIMIT 1",
                [],
                |row| row.get(0),
            ),
        };

        result.ok()
    }

    fn latest_failure_reason(&self, user_id: Option<&str>) -> Option<String> {
        let sql = match user_id {
            Some(_) => {
                "SELECT error_message
                 FROM push_delivery_attempts
                 WHERE user_id = ?1 AND outcome = 'failure'
                 ORDER BY created_at DESC
                 LIMIT 1"
            }
            None => {
                "SELECT error_message
                 FROM push_delivery_attempts
                 WHERE outcome = 'failure'
                 ORDER BY created_at DESC
                 LIMIT 1"
            }
        };

        let result = match user_id {
            Some(uid) => self.db.conn().query_row(sql, [uid], |row| row.get(0)),
            None => self.db.conn().query_row(sql, [], |row| row.get(0)),
        };

        result.ok()
    }
}

fn map_attempt_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PushDeliveryAttempt> {
    Ok(PushDeliveryAttempt {
        id: row.get(0)?,
        user_id: row.get(1)?,
        session_id: row.get(2)?,
        endpoint_hash: row.get(3)?,
        provider_host: row.get(4)?,
        event_type: row.get(5)?,
        outcome: row.get(6)?,
        http_status: row.get(7)?,
        error_message: row.get(8)?,
        latency_ms: row.get(9)?,
        created_at: row.get(10)?,
    })
}

fn endpoint_hash(endpoint: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(endpoint.as_bytes());
    let digest = hasher.finalize();
    hex_encode(digest.as_slice())
}

fn provider_host(endpoint: &str) -> String {
    Url::parse(endpoint)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX_DIGITS[(b >> 4) as usize] as char);
        out.push(HEX_DIGITS[(b & 0x0f) as usize] as char);
    }
    out
}
