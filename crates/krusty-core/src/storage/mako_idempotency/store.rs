use std::io::{Error as IoError, ErrorKind};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row, Transaction, TransactionBehavior};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::mako::canonical_timestamp;
use crate::storage::Database;

use super::{IdempotencyClaim, IdempotencyRecord};

const COLUMNS: &str = "scope_key, operation, idempotency_key, request_hash, resource_id, response_json, created_at, expires_at";

pub struct MakoIdempotencyStore {
    db: Database,
}

impl MakoIdempotencyStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Atomically reserves a key or returns the existing request's disposition.
    pub fn claim(
        &self,
        scope_key: &str,
        operation: &str,
        key: &str,
        request_hash: &str,
        now: DateTime<Utc>,
        ttl: Duration,
    ) -> Result<IdempotencyClaim> {
        validate_identity(scope_key, operation, key, request_hash)?;
        anyhow::ensure!(!ttl.is_zero(), "idempotency TTL is zero");
        let expires_at = now
            .checked_add_signed(chrono::Duration::from_std(ttl).context("TTL is too large")?)
            .context("idempotency expiry overflow")?;
        let now = canonical_timestamp(now);
        let expires_at = canonical_timestamp(expires_at);
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;

        let existing = get_in_transaction(&tx, scope_key, operation, key)?;
        if let Some(existing) = existing {
            if existing.expires_at > now {
                tx.commit()?;
                if existing.request_hash != request_hash {
                    return Ok(IdempotencyClaim::Conflict {
                        existing_request_hash: existing.request_hash,
                    });
                }
                return Ok(if existing.response.is_some() {
                    IdempotencyClaim::Replay(existing)
                } else {
                    IdempotencyClaim::InProgress(existing)
                });
            }
            tx.execute(
                "DELETE FROM mako_idempotency_keys
                 WHERE scope_key = ?1 AND operation = ?2 AND idempotency_key = ?3",
                params![scope_key, operation, key],
            )?;
        }

        tx.execute(
            "INSERT INTO mako_idempotency_keys (
                scope_key, operation, idempotency_key, request_hash, resource_id,
                response_json, created_at, expires_at
             ) VALUES (?1, ?2, ?3, ?4, NULL, NULL, ?5, ?6)",
            params![scope_key, operation, key, request_hash, now, expires_at],
        )?;
        let record = get_in_transaction(&tx, scope_key, operation, key)?
            .context("idempotency claim disappeared inside transaction")?;
        tx.commit()?;
        Ok(IdempotencyClaim::Claimed(record))
    }

    /// Publishes the canonical result. A stale, expired, conflicting, or duplicate owner gets false.
    pub fn complete(
        &self,
        scope_key: &str,
        operation: &str,
        key: &str,
        request_hash: &str,
        resource_id: Option<&str>,
        response: &Value,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        validate_identity(scope_key, operation, key, request_hash)?;
        let response_json = serde_json::to_string(response)?;
        let changed = self.db.conn().execute(
            "UPDATE mako_idempotency_keys
             SET resource_id = ?5, response_json = ?6
             WHERE scope_key = ?1 AND operation = ?2 AND idempotency_key = ?3
               AND request_hash = ?4 AND response_json IS NULL AND expires_at > ?7",
            params![
                scope_key,
                operation,
                key,
                request_hash,
                resource_id,
                response_json,
                canonical_timestamp(now),
            ],
        )?;
        Ok(changed == 1)
    }

    pub fn get(
        &self,
        scope_key: &str,
        operation: &str,
        key: &str,
    ) -> Result<Option<IdempotencyRecord>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM mako_idempotency_keys
             WHERE scope_key = ?1 AND operation = ?2 AND idempotency_key = ?3"
        );
        self.db
            .conn()
            .query_row(&sql, params![scope_key, operation, key], map_record)
            .optional()
            .context("reading Mako idempotency record")
    }

    pub fn release(
        &self,
        scope_key: &str,
        operation: &str,
        key: &str,
        request_hash: &str,
    ) -> Result<bool> {
        validate_identity(scope_key, operation, key, request_hash)?;
        let changed = self.db.conn().execute(
            "DELETE FROM mako_idempotency_keys
             WHERE scope_key = ?1 AND operation = ?2 AND idempotency_key = ?3
               AND request_hash = ?4 AND response_json IS NULL",
            params![scope_key, operation, key, request_hash],
        )?;
        Ok(changed == 1)
    }

    pub fn delete_expired(&self, now: DateTime<Utc>) -> Result<usize> {
        let changed = self.db.conn().execute(
            "DELETE FROM mako_idempotency_keys WHERE expires_at <= ?1",
            [canonical_timestamp(now)],
        )?;
        Ok(changed)
    }
}

pub fn hash_request_bytes(request: impl AsRef<[u8]>) -> String {
    let digest = Sha256::digest(request.as_ref());
    format!("{digest:x}")
}

fn get_in_transaction(
    tx: &Transaction<'_>,
    scope_key: &str,
    operation: &str,
    key: &str,
) -> Result<Option<IdempotencyRecord>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM mako_idempotency_keys
         WHERE scope_key = ?1 AND operation = ?2 AND idempotency_key = ?3"
    );
    tx.query_row(&sql, params![scope_key, operation, key], map_record)
        .optional()
        .context("reading idempotency record in transaction")
}

fn map_record(row: &Row<'_>) -> rusqlite::Result<IdempotencyRecord> {
    let response_json = row.get::<_, Option<String>>(5)?;
    let response = response_json
        .map(|json| {
            serde_json::from_str(&json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    5,
                    rusqlite::types::Type::Text,
                    Box::new(IoError::new(
                        ErrorKind::InvalidData,
                        format!("invalid idempotency response JSON: {error}"),
                    )),
                )
            })
        })
        .transpose()?;
    Ok(IdempotencyRecord {
        scope_key: row.get(0)?,
        operation: row.get(1)?,
        key: row.get(2)?,
        request_hash: row.get(3)?,
        resource_id: row.get(4)?,
        response,
        created_at: row.get(6)?,
        expires_at: row.get(7)?,
    })
}

fn validate_identity(scope: &str, operation: &str, key: &str, request_hash: &str) -> Result<()> {
    anyhow::ensure!(!scope.trim().is_empty(), "idempotency scope is empty");
    anyhow::ensure!(
        !operation.trim().is_empty(),
        "idempotency operation is empty"
    );
    anyhow::ensure!(!key.trim().is_empty(), "idempotency key is empty");
    anyhow::ensure!(
        !request_hash.trim().is_empty(),
        "idempotency request hash is empty"
    );
    Ok(())
}
