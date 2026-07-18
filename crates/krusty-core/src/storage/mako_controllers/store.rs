use std::io::{Error as IoError, ErrorKind};

use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension, Row};

use crate::mako::{normalize_timestamp, parse_timezone};
use crate::storage::Database;

use super::{MakoController, MakoControllerStatus};

const CONTROLLER_COLUMNS: &str = "id, scope_key, user_id, session_id, status, timezone, max_concurrent_runs, created_at, updated_at";

pub struct MakoControllerStore {
    db: Database,
}

impl MakoControllerStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub fn insert(&self, controller: &MakoController) -> Result<()> {
        parse_timezone(&controller.timezone)?;
        anyhow::ensure!(!controller.id.trim().is_empty(), "controller id is empty");
        anyhow::ensure!(
            !controller.scope_key.trim().is_empty(),
            "controller scope is empty"
        );
        anyhow::ensure!(
            !controller.session_id.trim().is_empty(),
            "controller session id is empty"
        );
        anyhow::ensure!(
            controller.max_concurrent_runs > 0,
            "max_concurrent_runs must be positive"
        );
        let created_at = normalize_timestamp(&controller.created_at)?;
        let updated_at = normalize_timestamp(&controller.updated_at)?;
        self.db
            .conn()
            .execute(
                "INSERT INTO mako_controllers (
                    id, scope_key, user_id, session_id, status, timezone,
                    max_concurrent_runs, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    controller.id,
                    controller.scope_key,
                    controller.user_id,
                    controller.session_id,
                    controller.status.to_string(),
                    controller.timezone,
                    controller.max_concurrent_runs,
                    created_at,
                    updated_at,
                ],
            )
            .context("inserting Mako controller")?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<Option<MakoController>> {
        self.query_one("id", id)
    }

    pub fn get_by_scope(&self, scope_key: &str) -> Result<Option<MakoController>> {
        self.query_one("scope_key", scope_key)
    }

    pub fn get_by_session(&self, session_id: &str) -> Result<Option<MakoController>> {
        self.query_one("session_id", session_id)
    }

    pub fn set_status(
        &self,
        id: &str,
        status: MakoControllerStatus,
        updated_at: &str,
    ) -> Result<bool> {
        let updated_at = normalize_timestamp(updated_at)?;
        let changed = self.db.conn().execute(
            "UPDATE mako_controllers SET status = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, status.to_string(), updated_at],
        )?;
        Ok(changed == 1)
    }

    pub fn set_concurrency(
        &self,
        id: &str,
        max_concurrent_runs: u32,
        updated_at: &str,
    ) -> Result<bool> {
        anyhow::ensure!(max_concurrent_runs > 0, "concurrency must be positive");
        let updated_at = normalize_timestamp(updated_at)?;
        let changed = self.db.conn().execute(
            "UPDATE mako_controllers
             SET max_concurrent_runs = ?2, updated_at = ?3
             WHERE id = ?1",
            params![id, max_concurrent_runs, updated_at],
        )?;
        Ok(changed == 1)
    }

    fn query_one(&self, column: &str, value: &str) -> Result<Option<MakoController>> {
        debug_assert!(matches!(column, "id" | "scope_key" | "session_id"));
        let sql = format!("SELECT {CONTROLLER_COLUMNS} FROM mako_controllers WHERE {column} = ?1");
        self.db
            .conn()
            .query_row(&sql, [value], map_controller)
            .optional()
            .context("reading Mako controller")
    }

    #[cfg(test)]
    pub(super) fn conn(&self) -> &rusqlite::Connection {
        self.db.conn()
    }
}

fn map_controller(row: &Row<'_>) -> rusqlite::Result<MakoController> {
    let status: String = row.get(4)?;
    Ok(MakoController {
        id: row.get(0)?,
        scope_key: row.get(1)?,
        user_id: row.get(2)?,
        session_id: row.get(3)?,
        status: MakoControllerStatus::parse(&status).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                Box::new(IoError::new(
                    ErrorKind::InvalidData,
                    format!("invalid Mako controller status: {status}"),
                )),
            )
        })?,
        timezone: row.get(5)?,
        max_concurrent_runs: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}
