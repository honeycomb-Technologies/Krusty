use std::io::{Error as IoError, ErrorKind};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row, Transaction, TransactionBehavior};

use crate::mako::canonical_timestamp;
use crate::storage::Database;

use super::{DaemonLease, DaemonLeaseAcquire};

const COLUMNS: &str = "lease_name, owner_id, fencing_token, acquired_at, heartbeat_at, expires_at";

pub struct MakoDaemonLeaseStore {
    db: Database,
}

impl MakoDaemonLeaseStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub fn acquire(
        &self,
        lease_name: &str,
        owner_id: &str,
        now: DateTime<Utc>,
        duration: Duration,
    ) -> Result<DaemonLeaseAcquire> {
        validate(lease_name, owner_id, duration)?;
        let expires = now
            .checked_add_signed(chrono::Duration::from_std(duration).context("lease is too long")?)
            .context("daemon lease expiry overflow")?;
        let now = canonical_timestamp(now);
        let expires = canonical_timestamp(expires);
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let existing = get_in_transaction(&tx, lease_name)?;

        match existing {
            None => {
                tx.execute(
                    "INSERT INTO mako_daemon_leases (
                        lease_name, owner_id, fencing_token, acquired_at, heartbeat_at, expires_at
                     ) VALUES (?1, ?2, 1, ?3, ?3, ?4)",
                    params![lease_name, owner_id, now, expires],
                )?;
            }
            Some(existing) if existing.owner_id == owner_id && existing.expires_at > now => {
                tx.execute(
                    "UPDATE mako_daemon_leases
                     SET heartbeat_at = ?3, expires_at = ?4
                     WHERE lease_name = ?1 AND owner_id = ?2 AND fencing_token = ?5",
                    params![lease_name, owner_id, now, expires, existing.fencing_token],
                )?;
            }
            Some(existing) if existing.expires_at <= now => {
                let fencing_token = existing
                    .fencing_token
                    .checked_add(1)
                    .context("daemon fencing token exhausted")?;
                anyhow::ensure!(
                    fencing_token <= i64::MAX as u64,
                    "daemon fencing token exceeds SQLite range"
                );
                tx.execute(
                    "UPDATE mako_daemon_leases
                     SET owner_id = ?2, fencing_token = ?3, acquired_at = ?4,
                         heartbeat_at = ?4, expires_at = ?5
                     WHERE lease_name = ?1 AND fencing_token = ?6",
                    params![
                        lease_name,
                        owner_id,
                        fencing_token,
                        now,
                        expires,
                        existing.fencing_token
                    ],
                )?;
            }
            Some(existing) => {
                tx.commit()?;
                return Ok(DaemonLeaseAcquire::HeldByOther {
                    owner_id: existing.owner_id,
                    expires_at: existing.expires_at,
                });
            }
        }

        let lease = get_in_transaction(&tx, lease_name)?
            .context("daemon lease disappeared inside acquisition transaction")?;
        tx.commit()?;
        Ok(DaemonLeaseAcquire::Acquired(lease))
    }

    pub fn heartbeat(
        &self,
        lease_name: &str,
        owner_id: &str,
        fencing_token: u64,
        now: DateTime<Utc>,
        duration: Duration,
    ) -> Result<bool> {
        validate(lease_name, owner_id, duration)?;
        let expires = now
            .checked_add_signed(chrono::Duration::from_std(duration).context("lease is too long")?)
            .context("daemon lease expiry overflow")?;
        let now = canonical_timestamp(now);
        let expires = canonical_timestamp(expires);
        let changed = self.db.conn().execute(
            "UPDATE mako_daemon_leases
             SET heartbeat_at = ?4, expires_at = ?5
             WHERE lease_name = ?1 AND owner_id = ?2 AND fencing_token = ?3
               AND expires_at > ?4",
            params![lease_name, owner_id, fencing_token, now, expires],
        )?;
        Ok(changed == 1)
    }

    pub fn release(&self, lease_name: &str, owner_id: &str, fencing_token: u64) -> Result<bool> {
        let changed = self.db.conn().execute(
            "UPDATE mako_daemon_leases
             SET expires_at = '0001-01-01T00:00:00.000000Z'
             WHERE lease_name = ?1 AND owner_id = ?2 AND fencing_token = ?3",
            params![lease_name, owner_id, fencing_token],
        )?;
        Ok(changed == 1)
    }

    pub fn get(&self, lease_name: &str) -> Result<Option<DaemonLease>> {
        let sql = format!("SELECT {COLUMNS} FROM mako_daemon_leases WHERE lease_name = ?1");
        self.db
            .conn()
            .query_row(&sql, [lease_name], map_lease)
            .optional()
            .context("reading Mako daemon lease")
    }
}

fn get_in_transaction(tx: &Transaction<'_>, lease_name: &str) -> Result<Option<DaemonLease>> {
    let sql = format!("SELECT {COLUMNS} FROM mako_daemon_leases WHERE lease_name = ?1");
    tx.query_row(&sql, [lease_name], map_lease)
        .optional()
        .context("reading Mako daemon lease in transaction")
}

fn map_lease(row: &Row<'_>) -> rusqlite::Result<DaemonLease> {
    let raw_token = row.get::<_, i64>(2)?;
    let fencing_token = u64::try_from(raw_token).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Integer,
            Box::new(IoError::new(
                ErrorKind::InvalidData,
                "negative daemon fencing token",
            )),
        )
    })?;
    Ok(DaemonLease {
        lease_name: row.get(0)?,
        owner_id: row.get(1)?,
        fencing_token,
        acquired_at: row.get(3)?,
        heartbeat_at: row.get(4)?,
        expires_at: row.get(5)?,
    })
}

fn validate(lease_name: &str, owner_id: &str, duration: Duration) -> Result<()> {
    anyhow::ensure!(!lease_name.trim().is_empty(), "daemon lease name is empty");
    anyhow::ensure!(!owner_id.trim().is_empty(), "daemon lease owner is empty");
    anyhow::ensure!(!duration.is_zero(), "daemon lease duration is zero");
    Ok(())
}
